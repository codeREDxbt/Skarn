// Selora IDE — Phase Z4 editor store.
//
// The protocol owns open buffers; this store is the thin, observable view of
// `bootstrap.buffers` that the editor region renders. Two pieces of state are
// local by design: which buffer is active in the pane and a one-shot reveal
// request (path + 1-based line) issued by search, diagnostics, or the minimap.
import { currentEnvelopeContext, getAppSnapshot, subscribeApp } from '../../app/app-store';
import {
  closeBuffer as closeBufferCommand,
  keepBufferEdits as keepBufferEditsCommand,
  openFile as openFileCommand,
  reloadBufferFromDisk as reloadBufferFromDiskCommand,
  saveBuffer as saveBufferCommand,
  setActiveBuffer as setActiveBufferCommand,
  type BootstrapState,
  type BufferSnapshot,
  type EditorContextDto,
  type EnvelopeContext,
  type FileTreeNode,
  type Selection,
} from '../../bridge/bridge';
import { toastStore } from '../toasts/toast-store';
import { beginEditorClose, finishEditorClose, hasPendingBufferEdits, isEditorClosing, isEditorPathClosing, reconcileEditorCloses, reconcilePendingEditorEdits, rejectEditorClose, retryPendingEditorEdits } from './pending-edits';

export type EditorBlockingIssueKind = 'unsynchronized' | 'close_locked';

export type EditorBlockingIssue = {
  bufferId: string;
  path: string;
  filename: string;
  kind: EditorBlockingIssueKind;
  summary: string;
  detail: string;
  actionLabel: 'Retry sync' | 'Retry close';
};

export type EditorTab = {
  id: string;
  path: string;
  filename: string;
  language: string;
  isDirty: boolean;
  /** P2 gap 3: the file also changed on disk (local edits were kept). */
  externallyChanged: boolean;
  /** Persistent blocking error (unsynchronized edits or unconfirmed close lock). */
  blockingIssue: EditorBlockingIssue | null;
  version: number;
  /** Single-click preview tab: replaced by the next single-click open. */
  isPreview: boolean;
};

export type RevealTarget = {
  path: string;
  line: number;
  requestId: number;
};

export type EditorStoreState = {
  tabs: EditorTab[];
  activeBufferId: string | null;
  activeBuffer: BufferSnapshot | null;
  activeSelection: Selection | null;
  revealTarget: RevealTarget | null;
  /** Buffers the user marked "Keep Open"; bulk closes spare them. */
  pinnedIds: string[];
  /** Pinned tab paths; paths survive bootstrap buffer-id churn. */
  pinnedPaths: string[];
  /** Path of the single preview tab, or null when every tab is permanent. */
  previewPath: string | null;
  workspaceRoot: string | null;
  workspaceName: string | null;
  fileTree: FileTreeNode | null;
  /** P2 gap 3: buffer id → path for tabs with an unresolved disk conflict. */
  conflicts: Record<string, string>;
  /** Persistent blocking file issues surfaced on tab, Problems, and StatusBar. */
  blockingIssues: EditorBlockingIssue[];
};

type Listener = () => void;

const listeners = new Set<Listener>();
let localActiveId: string | null = null;
let revealTarget: RevealTarget | null = null;
let revealSequence = 0;
let appSnapshot = getAppSnapshot();
let protocolActiveId: string | null = appSnapshot.bootstrap?.active_buffer_id ?? null;
let pendingOpenPath: string | null = null;
let knownBufferIds = new Set(
  (appSnapshot.bootstrap?.buffers ?? []).map((buffer) => buffer.buffer_id),
);
// Selections typed in the pane, keyed by buffer, recorded before the protocol
// echo lands. Kept outside the snapshot so a keystroke does not rebuild the app.
let liveSelections = new Map<string, Selection>();
// "Keep Open" is a local tab affordance: the protocol has no pin concept, so
// the pins survive snapshots and are pruned whenever a buffer disappears.
// Pins are tracked by path because bootstrap reassigns buffer ids.
let pinnedPaths = new Set<string>();
// The preview tab is the transient tab a single click in the explorer opens.
// At most one tab is a preview; opening another replaces it.
let previewPath: string | null = null;
const blockingIssuesById = new Map<string, EditorBlockingIssue>();
let state: EditorStoreState = deriveState(appSnapshot, localActiveId, revealTarget);
let appUnsubscribe: (() => void) | null = null;

function setBlockingIssue(
  bufferId: string,
  path: string,
  kind: EditorBlockingIssueKind,
): void {
  const filename = filenameOf(path);
  const issue: EditorBlockingIssue =
    kind === 'close_locked'
      ? {
          bufferId,
          path,
          filename,
          kind,
          summary: 'Close unconfirmed',
          detail: `${filename} did not confirm closing. Editing is paused to protect your contents.`,
          actionLabel: 'Retry close',
        }
      : {
          bufferId,
          path,
          filename,
          kind,
          summary: 'Unsynchronized edits',
          detail: `Edits in ${filename} did not reach the workspace. Keep this tab open — your local text is safe.`,
          actionLabel: 'Retry sync',
        };
  blockingIssuesById.set(bufferId, issue);
}

function clearBlockingIssue(bufferId: string): void {
  blockingIssuesById.delete(bufferId);
}

function reconcileBlockingIssues(bootstrap: BootstrapState | null): void {
  if (!bootstrap) {
    blockingIssuesById.clear();
    return;
  }
  const buffers = new Set(bootstrap.buffers.map((buffer) => buffer.buffer_id));
  for (const [id, issue] of blockingIssuesById) {
    if (!buffers.has(id)) {
      blockingIssuesById.delete(id);
      continue;
    }
    if (issue.kind === 'unsynchronized' && !hasPendingBufferEdits(bootstrap, id)) {
      blockingIssuesById.delete(id);
    } else if (issue.kind === 'close_locked' && !isEditorClosing(id)) {
      blockingIssuesById.delete(id);
    }
  }
}

function filenameOf(path: string): string {
  const segments = path.split(/[\\/]/);
  return segments[segments.length - 1] || path;
}

function pathsEqual(left: string, right: string): boolean {
  return left.replace(/\\/g, '/').toLowerCase() === right.replace(/\\/g, '/').toLowerCase();
}

function isPinnedPath(path: string): boolean {
  for (const pinned of pinnedPaths) if (pathsEqual(pinned, path)) return true;
  return false;
}

function forgetPinnedPath(path: string): void {
  for (const pinned of pinnedPaths) if (pathsEqual(pinned, path)) pinnedPaths.delete(pinned);
}

/**
 * A preview tab stops being a preview once it is edited or closed: the user
 * invested in it, so the next single click must not replace it.
 */
function reconcilePreview(): void {
  if (!previewPath) return;
  if (pendingOpenPath && pathsEqual(pendingOpenPath, previewPath)) return;
  const tab = state.tabs.find((candidate) => pathsEqual(candidate.path, previewPath!));
  if (!tab || tab.isDirty) previewPath = null;
}

function isEmptySelection(selection: Selection): boolean {
  return (
    selection.anchor.line === selection.active.line &&
    selection.anchor.utf16_column === selection.active.utf16_column
  );
}

function deriveState(
  app: ReturnType<typeof getAppSnapshot>,
  preferredActiveId: string | null,
  reveal: RevealTarget | null,
): EditorStoreState {
  const bootstrap: BootstrapState | null = app.bootstrap;
  reconcileBlockingIssues(bootstrap);
  const buffers = bootstrap?.buffers ?? [];
  const conflicts = app.bufferConflicts ?? {};
  const tabs: EditorTab[] = buffers.map((buffer) => ({
    id: buffer.buffer_id,
    path: buffer.path,
    filename: filenameOf(buffer.path),
    language: buffer.language,
    isDirty: buffer.dirty,
    externallyChanged: Object.prototype.hasOwnProperty.call(conflicts, buffer.buffer_id),
    blockingIssue: blockingIssuesById.get(buffer.buffer_id) ?? null,
    version: buffer.version,
    isPreview: Boolean(previewPath && pathsEqual(previewPath, buffer.path)),
  }));

  let activeBufferId: string | null = null;
  if (preferredActiveId && buffers.some((buffer) => buffer.buffer_id === preferredActiveId)) {
    activeBufferId = preferredActiveId;
  } else if (
    bootstrap?.active_buffer_id &&
    buffers.some((buffer) => buffer.buffer_id === bootstrap.active_buffer_id)
  ) {
    activeBufferId = bootstrap.active_buffer_id;
  } else {
    activeBufferId = tabs[0]?.id ?? null;
  }

  const activeBuffer = buffers.find((buffer) => buffer.buffer_id === activeBufferId) ?? null;

  // The pane knows the caret before the protocol echo arrives, so a live
  // selection wins over the snapshot. Empty selections stay out of state.
  const protocolSelection = activeBuffer?.selection ?? null;
  const liveSelection = activeBufferId ? liveSelections.get(activeBufferId) ?? null : null;
  const candidate = liveSelection ?? protocolSelection;
  const activeSelection = candidate && !isEmptySelection(candidate) ? candidate : null;

  return {
    tabs,
    activeBufferId,
    activeBuffer,
    activeSelection,
    revealTarget: reveal,
    pinnedIds: tabs.filter((tab) => isPinnedPath(tab.path)).map((tab) => tab.id),
    pinnedPaths: tabs.filter((tab) => isPinnedPath(tab.path)).map((tab) => tab.path),
    previewPath: tabs.some((tab) => tab.isPreview) ? previewPath : null,
    workspaceRoot: bootstrap?.workspace.root ?? null,
    workspaceName: bootstrap?.workspace.display_name ?? null,
    fileTree: bootstrap?.file_tree.root ?? null,
    conflicts,
    blockingIssues: tabs
      .map((tab) => tab.blockingIssue)
      .filter((issue): issue is EditorBlockingIssue => issue !== null),
  };
}

function notify(): void {
  for (const listener of listeners) listener();
}

function rebuild(): void {
  focusPendingBuffer();
  state = deriveState(appSnapshot, localActiveId, revealTarget);
  localActiveId = state.activeBufferId;
  const previewBefore = previewPath;
  reconcilePreview();
  if (previewPath !== previewBefore) state = deriveState(appSnapshot, localActiveId, revealTarget);
  notify();
}

function focusPendingBuffer(): void {
  const pending = pendingOpenPath;
  const buffers = appSnapshot.bootstrap?.buffers ?? [];
  if (pending) {
    const opened = buffers.find((buffer) => pathsEqual(buffer.path, pending));
    if (opened) {
      localActiveId = opened.buffer_id;
      pendingOpenPath = null;
    }
  }

  const nextProtocolActiveId = appSnapshot.bootstrap?.active_buffer_id ?? null;
  if (
    nextProtocolActiveId &&
    nextProtocolActiveId !== protocolActiveId &&
    buffers.some((buffer) => buffer.buffer_id === nextProtocolActiveId)
  ) {
    localActiveId = nextProtocolActiveId;
  }

  const added = buffers.filter((buffer) => !knownBufferIds.has(buffer.buffer_id));
  if (!pending && added.length === 1 && nextProtocolActiveId === protocolActiveId) {
    localActiveId = added[0].buffer_id;
  }
  protocolActiveId = nextProtocolActiveId;
  knownBufferIds = new Set(buffers.map((buffer) => buffer.buffer_id));
}

function syncFromApp(): void {
  const next = getAppSnapshot();
  if (next === appSnapshot) return;
  appSnapshot = next;
  reconcilePendingEditorEdits(next.bootstrap);
  reconcileEditorCloses(next.bootstrap);
  focusPendingBuffer();
  state = deriveState(appSnapshot, localActiveId, revealTarget);
  localActiveId = state.activeBufferId;
  const previewBefore = previewPath;
  reconcilePreview();
  if (previewPath !== previewBefore) state = deriveState(appSnapshot, localActiveId, revealTarget);
}

function ensureAppSubscription(): void {
  if (appUnsubscribe) return;
  appUnsubscribe = subscribeApp(() => {
    const next = getAppSnapshot();
    if (next === appSnapshot) return;
    appSnapshot = next;
    reconcilePendingEditorEdits(next.bootstrap);
    reconcileEditorCloses(next.bootstrap);
    rebuild();
  });
}

function openFileAt(path: string, context: EnvelopeContext): void {
  pendingOpenPath = path;
  void openFileCommand(path, context).catch(() => {
    if (pendingOpenPath === path) pendingOpenPath = null;
  });
}

function reportMissingContext(): void {
  toastStore.pushLocal('warning', 'Connect to a workspace before editing files.');
}

type CloseRejection = 'ambiguous' | 'still-open' | 'closed';

/**
 * Classify a rejected close command. Only the bridge's protocol error shape
 * (code + message) is a definite backend decision; anything else, including a
 * raw transport failure, stays ambiguous and keeps the editor locked.
 */
function classifyCloseRejection(cause: unknown): CloseRejection {
  if (typeof cause !== 'object' || cause === null) return 'ambiguous';
  const candidate = cause as { code?: unknown; message?: unknown };
  if (typeof candidate.code !== 'string' || typeof candidate.message !== 'string') return 'ambiguous';
  return candidate.code === 'unknown_buffer' ? 'closed' : 'still-open';
}

function closeWhenSynchronized(id: string, context: EnvelopeContext): boolean {
  const bootstrap = getAppSnapshot().bootstrap;
  const targetPath = bootstrap?.buffers.find((candidate) => candidate.buffer_id === id)?.path ?? id;
  if (!bootstrap || !beginEditorClose(bootstrap, id)) return false;
  void closeBufferCommand(id, context).then(() => {
    finishEditorClose(id, true);
    clearBlockingIssue(id);
    reconcileEditorCloses(getAppSnapshot().bootstrap);
    rebuild();
  }).catch((cause: unknown) => {
    const rejection = classifyCloseRejection(cause);
    rejectEditorClose(id, rejection);
    if (rejection === 'ambiguous') {
      finishEditorClose(id, false);
      setBlockingIssue(id, targetPath, 'close_locked');
      rebuild();
      return;
    }
    clearBlockingIssue(id);
    reconcileEditorCloses(getAppSnapshot().bootstrap);
    rebuild();
    if (rejection === 'still-open') {
      toastStore.pushLocal('warning', 'The workspace rejected the close. The file is still open; retry when the workspace is ready.');
    }
  });
  return true;
}

/** Replace the transient preview tab with the file the user just opened. */
function dismissPreviewFor(context: EnvelopeContext, nextPath: string): void {
  const previous = state.tabs.find(
    (tab) => tab.isPreview && !pathsEqual(tab.path, nextPath) && !tab.isDirty && !isPinnedPath(tab.path),
  );
  if (previous) closeWhenSynchronized(previous.id, context);
}

/**
 * Tell Rust which tab the user is looking at, so the next bootstrap's
 * `active_buffer_id` is the real selection and not path order. Fire and
 * forget: the pane already applied the switch locally; a failed ack leaves
 * the previous (still valid) selection.
 */
function recordActiveBuffer(bufferId: string): void {
  const context = currentEnvelopeContext();
  if (!context) return;
  void setActiveBufferCommand(bufferId, context).catch(() => undefined);
}

export function currentEditorContext(): EnvelopeContext | null {
  return currentEnvelopeContext();
}

/** Record the editor pane's live selection (before the protocol echo lands). */
export function recordEditorSelection(bufferId: string, selection: Selection): void {
  const previous = state.activeSelection;
  if (isEmptySelection(selection)) {
    liveSelections.delete(bufferId);
  } else {
    liveSelections.set(bufferId, selection);
  }
  state = deriveState(appSnapshot, localActiveId, revealTarget);
  if (state.activeSelection !== previous) notify();
}

/**
 * The active editor reference for the next agent turn, or null when no
 * buffer is active. Never includes file text: the agent reads it with
 * `read_file`. Empty selections are omitted, not sent as 0:0-0:0.
 */
export function activeEditorContext(): EditorContextDto | null {
  syncFromApp();
  const buffer = state.activeBuffer;
  if (!buffer || !buffer.path) return null;
  return {
    path: buffer.path,
    language: buffer.language,
    ...(state.activeSelection ? { selection: state.activeSelection } : {}),
    dirty: buffer.dirty,
  };
}

export const editorStore = {
  subscribe(listener: Listener): () => void {
    ensureAppSubscription();
    listeners.add(listener);
    syncFromApp();
    return () => {
      listeners.delete(listener);
    };
  },

  getSnapshot(): EditorStoreState {
    syncFromApp();
    return state;
  },

  /** Rebuild from the current bootstrap (rebootstrap / test isolation). */
  reset(): void {
    localActiveId = null;
    revealTarget = null;
    pendingOpenPath = null;
    liveSelections.clear();
    pinnedPaths = new Set();
    previewPath = null;
    blockingIssuesById.clear();
    appSnapshot = getAppSnapshot();
    reconcilePendingEditorEdits(appSnapshot.bootstrap);
    protocolActiveId = appSnapshot.bootstrap?.active_buffer_id ?? null;
    knownBufferIds = new Set(
      (appSnapshot.bootstrap?.buffers ?? []).map((buffer) => buffer.buffer_id),
    );
    state = deriveState(appSnapshot, localActiveId, revealTarget);
    localActiveId = state.activeBufferId;
    notify();
  },

  setActive(id: string): void {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return;
    if (localActiveId === id) return;
    localActiveId = id;
    recordActiveBuffer(id);
    rebuild();
    // Tell Rust which tab is active so a later bootstrap reports this one
    // instead of the path-sorted first buffer.
    const context = currentEditorContext();
    if (context) {
      void setActiveBufferCommand(id, context).catch(() => undefined);
    }
  },

  async openPath(path: string, options: { preview?: boolean } = {}): Promise<void> {
    syncFromApp();
    if (appSnapshot.bootstrap && isEditorPathClosing(appSnapshot.bootstrap.workspace.id, path)) return;
    const existing = state.tabs.find((tab) => pathsEqual(tab.path, path));
    if (existing) {
      // Opening the file permanently promotes a preview tab in place.
      if (options.preview === false && existing.isPreview) {
        previewPath = null;
        rebuild();
      }
      editorStore.setActive(existing.id);
      return;
    }
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    if (options.preview === false) {
      previewPath = null;
    } else {
      dismissPreviewFor(context, path);
      previewPath = path;
    }
    pendingOpenPath = path;
    await openFileCommand(path, context).catch(() => {
      if (pendingOpenPath === path) pendingOpenPath = null;
    });
  },

  async openPathAtLine(path: string, line: number, options: { preview?: boolean } = {}): Promise<void> {
    syncFromApp();
    if (appSnapshot.bootstrap && isEditorPathClosing(appSnapshot.bootstrap.workspace.id, path)) return;
    const existing = state.tabs.find((tab) => pathsEqual(tab.path, path));
    if (existing) {
      // A permanent open promotes the tab; a preview open leaves it as-is.
      if (options.preview === false && existing.isPreview) previewPath = null;
      localActiveId = existing.id;
      recordActiveBuffer(existing.id);
    } else {
      const context = currentEditorContext();
      if (!context) {
        reportMissingContext();
        return;
      }
      if (options.preview === false) {
        previewPath = null;
      } else {
        dismissPreviewFor(context, path);
        previewPath = path;
      }
      openFileAt(path, context);
    }
    revealSequence += 1;
    revealTarget = { path, line, requestId: revealSequence };
    rebuild();
  },

  clearReveal(requestId?: number): void {
    if (!revealTarget) return;
    if (requestId !== undefined && revealTarget.requestId !== requestId) return;
    revealTarget = null;
    rebuild();
  },

  closeBuffer(id: string, discardDirty = false): void {
    syncFromApp();
    const tab = state.tabs.find((candidate) => candidate.id === id);
    if (!tab || (tab.isDirty && !discardDirty)) {
      if (tab) toastStore.pushLocal('warning', 'Save or explicitly discard the unsaved changes before closing this file.');
      return;
    }
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    const bootstrap = appSnapshot.bootstrap;
    if (bootstrap && hasPendingBufferEdits(bootstrap, id)) {
      // A failed edit must not block close forever. Recover it against the
      // current backend version first; the recovered buffer may now be dirty,
      // so never discard it in the same click.
      void retryPendingEditorEdits(bootstrap, id).then((recovered) => {
        if (!recovered) {
          setBlockingIssue(id, tab.path, 'unsynchronized');
          rebuild();
          return;
        }
        clearBlockingIssue(id);
        rebuild();
        toastStore.pushLocal('info', 'Editor changes were recovered. Save the file, or close again to discard them.');
      });
      return;
    }
    if (!closeWhenSynchronized(id, context)) {
      setBlockingIssue(id, tab.path, isEditorClosing(id) ? 'close_locked' : 'unsynchronized');
      rebuild();
    }
  },

  /**
   * Close several buffers at once, skipping dirty and "kept open" tabs so no
   * unsaved edit is ever discarded silently. Returns how many closes were
   * dispatched and how many requested tabs were spared.
   */
  closeMany(ids: string[]): { closed: number; kept: number } {
    syncFromApp();
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return { closed: 0, kept: 0 };
    }
    const requested = new Set(ids);
    const matches = state.tabs.filter((tab) => requested.has(tab.id));
    const targets = matches.filter((tab) => !tab.isDirty && !isPinnedPath(tab.path));
    const closed = targets.filter((tab) => closeWhenSynchronized(tab.id, context)).length;
    return { closed, kept: matches.length - closed };
  },

  /** Restore the tab affordances saved with a session (pins + preview tab). */
  restoreTabAffordances(pinned: string[], preview: string | null): void {
    syncFromApp();
    pinnedPaths = new Set(pinned);
    previewPath = preview;
    rebuild();
  },

  /** Toggle the local "Keep Open" pin for one tab. */
  togglePinned(id: string): void {
    syncFromApp();
    const tab = state.tabs.find((candidate) => candidate.id === id);
    if (!tab) return;
    if (isPinnedPath(tab.path)) {
      forgetPinnedPath(tab.path);
    } else {
      pinnedPaths.add(tab.path);
      // Pinning a preview makes it a permanent tab.
      if (previewPath && pathsEqual(previewPath, tab.path)) previewPath = null;
    }
    rebuild();
  },

  /**
   * Save one buffer (which may not be active) and resolve `true` only once the
   * protocol acknowledges the save. The tab bar uses this to gate closing a
   * dirty tab: a rejected save must leave the buffer open.
   */
  async saveBuffer(id: string): Promise<boolean> {
    syncFromApp();
    const tab = state.tabs.find((candidate) => candidate.id === id);
    if (!tab) return false;
    const bootstrap = appSnapshot.bootstrap;
    if (bootstrap && hasPendingBufferEdits(bootstrap, id)) {
      // Saving is the user-visible retry: republish the local document at the
      // backend's current version instead of leaving the buffer locked.
      const recovered = await retryPendingEditorEdits(bootstrap, id);
      if (!recovered) {
        setBlockingIssue(id, tab.path, 'unsynchronized');
        rebuild();
        return false;
      }
      clearBlockingIssue(id);
      rebuild();
    }
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return false;
    }
    try {
      await saveBufferCommand(id, context);
      return true;
    } catch {
      return false;
    }
  },

  /** Retry recovering an unsynchronized buffer or confirming a locked close. */
  async retryBlockingIssue(id: string): Promise<boolean> {
    syncFromApp();
    const issue = blockingIssuesById.get(id);
    const tab = state.tabs.find((candidate) => candidate.id === id);
    const bootstrap = appSnapshot.bootstrap;
    if (!issue || !tab || !bootstrap) {
      clearBlockingIssue(id);
      rebuild();
      return true;
    }
    if (issue.kind === 'unsynchronized') {
      const recovered = await retryPendingEditorEdits(bootstrap, id);
      if (recovered) clearBlockingIssue(id);
      else setBlockingIssue(id, tab.path, 'unsynchronized');
      rebuild();
      return recovered;
    }
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return false;
    }
    try {
      await closeBufferCommand(id, context);
      finishEditorClose(id, true);
      clearBlockingIssue(id);
      reconcileEditorCloses(getAppSnapshot().bootstrap);
      rebuild();
      return true;
    } catch (cause: unknown) {
      const rejection = classifyCloseRejection(cause);
      rejectEditorClose(id, rejection);
      if (rejection === 'ambiguous') {
        setBlockingIssue(id, tab.path, 'close_locked');
        rebuild();
        return false;
      }
      clearBlockingIssue(id);
      reconcileEditorCloses(getAppSnapshot().bootstrap);
      rebuild();
      return true;
    }
  },

  saveActive(): void {
    syncFromApp();
    const active = state.activeBuffer;
    if (!active) return;
    void editorStore.saveBuffer(active.buffer_id);
  },

  /** P2 gap 3: take the disk text, discarding the kept local edits. */
  reloadFromDisk(id: string): void {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return;
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    void reloadBufferFromDiskCommand(id, context).catch(() => undefined);
  },

  /** P2 gap 3: keep the local text and clear the external-change marker. */
  keepMyEdits(id: string): void {
    syncFromApp();
    if (!state.tabs.some((tab) => tab.id === id)) return;
    const context = currentEditorContext();
    if (!context) {
      reportMissingContext();
      return;
    }
    void keepBufferEditsCommand(id, context).catch(() => undefined);
  },
};
