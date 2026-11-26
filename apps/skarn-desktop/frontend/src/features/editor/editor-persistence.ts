import { currentEnvelopeContext, getAppSnapshot, subscribeApp } from '../../app/app-store';
import { openFile, type BootstrapState, type Selection } from '../../bridge/bridge';
import { editorStore } from './editor-store';
import { localEditorPreferences } from './local-preferences';

type Session = {
  files: Array<{ path: string; selection: Selection | null }>;
  activePath: string | null;
  /** "Keep Open" pins, stored by path so they survive buffer-id churn. */
  pinnedPaths?: string[];
  /** The single transients tab opened by a single click, if any. */
  previewPath?: string | null;
};
const LAST_WORKSPACE = 'skarn:last-editor-workspace';
const keyFor = (root: string) => {
  const normalized = root.replace(/\\/g, '/');
  const windowsPath = /^[a-z]:\//i.test(normalized) || root.startsWith('\\\\');
  return 'skarn:editor-session:' + (windowsPath ? normalized.toLowerCase() : normalized);
};
const cursors = new Map<string, Selection>();
const cursorKey = (root: string, path: string) => keyFor(root) + ':' + path;

function readSession(root: string): Session | null {
  try {
    const value = JSON.parse(localStorage.getItem(keyFor(root)) ?? 'null');
    if (!value || !Array.isArray(value.files)) return null;
    return {
      files: value.files.filter((file: { path?: unknown }) => file && typeof file.path === 'string').slice(0, 100),
      activePath: typeof value.activePath === 'string' ? value.activePath : null,
      pinnedPaths: Array.isArray(value.pinnedPaths)
        ? value.pinnedPaths.filter((path: unknown): path is string => typeof path === 'string')
        : [],
      previewPath: typeof value.previewPath === 'string' ? value.previewPath : null,
    };
  } catch { return null; }
}

export function rememberEditorCursor(path: string, selection: Selection): void {
  const root = getAppSnapshot().bootstrap?.workspace.root;
  if (root) cursors.set(cursorKey(root, path), selection);
}

export function savedEditorSelection(path: string): Selection | null {
  if (!localEditorPreferences.getSnapshot().restoreSession) return null;
  const root = getAppSnapshot().bootstrap?.workspace.root;
  if (!root) return null;
  const selection = cursors.get(cursorKey(root, path)) ?? readSession(root)?.files.find((file) => file.path === path)?.selection;
  if (!selection || ![selection.anchor?.line, selection.anchor?.utf16_column, selection.active?.line, selection.active?.utf16_column]
    .every((value) => Number.isInteger(value) && value! >= 0)) return null;
  return selection;
}

/** Saves only file references and positions, never unsaved document contents. */
function persistSession(bootstrap: BootstrapState): void {
  const root = bootstrap.workspace.root;
  if (!root) return;
  const snapshot = editorStore.getSnapshot();
  // Only clean buffers are restored: a dirty tab has no backend copy of its
  // text, so reopening it after a restart would silently lose the edits.
  const cleanBuffers = bootstrap.buffers.filter((buffer) => !buffer.dirty);
  const cleanPaths = new Set(cleanBuffers.map((buffer) => buffer.path));
  const active = snapshot.activeBuffer;
  const session: Session = {
    files: cleanBuffers.map((buffer) => ({
      path: buffer.path,
      selection: cursors.get(cursorKey(root, buffer.path)) ?? buffer.selection,
    })),
    activePath: active && !active.dirty && cleanPaths.has(active.path) ? active.path : null,
    pinnedPaths: snapshot.pinnedPaths.filter((path) => cleanPaths.has(path)),
    previewPath:
      snapshot.previewPath && cleanPaths.has(snapshot.previewPath) ? snapshot.previewPath : null,
  };
  try {
    localStorage.setItem(keyFor(root), JSON.stringify(session));
    localStorage.setItem(LAST_WORKSPACE, root);
  } catch { /* Storage can be unavailable in a restricted WebView. */ }
}

export function startEditorPersistence(): () => void {
  let stopped = false;
  let generation = 0;
  let restoring: number | null = null;
  let lastRoot: string | null = null;
  const visited = new Set<string>();
  const timers = new Map<string, { version: number; timer: ReturnType<typeof setTimeout> }>();
  const saving = new Set<string>();
  const attempted = new Map<string, number>();
  const pendingWaits = new Set<() => void>();

  const waitForBuffer = (path: string, root: string): Promise<void> => new Promise((resolve) => {
    let unsubscribe = () => {};
    const finish = () => { clearTimeout(timer); unsubscribe(); pendingWaits.delete(finish); resolve(); };
    const check = () => {
      const bootstrap = getAppSnapshot().bootstrap;
      if (stopped || bootstrap?.workspace.root !== root || bootstrap.buffers.some((buffer) => buffer.path === path)) finish();
    };
    const timer = setTimeout(finish, 2000);
    pendingWaits.add(finish);
    unsubscribe = subscribeApp(check);
    check();
  });

  const restore = async (root: string, session: Session, token: number) => {
    restoring = token;
    for (const file of session.files) {
      if (file.selection && !cursors.has(cursorKey(root, file.path))) {
        cursors.set(cursorKey(root, file.path), file.selection);
      }
    }
    try {
      for (const file of session.files) {
        if (stopped || token !== generation || getAppSnapshot().bootstrap?.workspace.root !== root || !localEditorPreferences.getSnapshot().restoreSession) return;
        const context = currentEnvelopeContext();
        if (!context) return;
        if (getAppSnapshot().bootstrap?.buffers.some((buffer) => buffer.path === file.path)) continue;
        try { await openFile(file.path, context); await waitForBuffer(file.path, root); }
        catch { /* Missing or renamed files must not stop the rest of the session. */ }
      }
      if (stopped || token !== generation || getAppSnapshot().bootstrap?.workspace.root !== root || !localEditorPreferences.getSnapshot().restoreSession) return;
      // Restore tab affordances before selecting the active tab so a restored
      // preview tab is marked as a preview, not promoted by the selection.
      if ((session.pinnedPaths?.length ?? 0) > 0 || session.previewPath) {
        editorStore.restoreTabAffordances(session.pinnedPaths ?? [], session.previewPath ?? null);
      }
      const active = editorStore.getSnapshot().tabs.find((tab) => tab.path === session.activePath);
      if (active) editorStore.setActive(active.id);
    } finally { if (restoring === token) restoring = null; }
  };

  const sync = () => {
    const app = getAppSnapshot();
    const bootstrap = app.bootstrap;
    if (stopped || app.phase !== 'ready' || !bootstrap) return;
    const preferences = localEditorPreferences.getSnapshot();
    const root = bootstrap.workspace.root;
    if (root !== lastRoot) {
      for (const entry of timers.values()) clearTimeout(entry.timer);
      timers.clear(); attempted.clear(); lastRoot = root; generation++; restoring = null;
      for (const finish of pendingWaits) finish();
    }
    if (!restoring && root && !visited.has(root)) {
      visited.add(root);
      const session = preferences.restoreSession ? readSession(root) : null;
      if (session?.files.length) {
        void restore(root, session, generation).finally(sync);
        return;
      }
    }
    if (!restoring) persistSession(bootstrap);
    for (const [id, entry] of timers) {
      const buffer = bootstrap.buffers.find((buffer) => buffer.buffer_id === id);
      if (!preferences.autoSave || !buffer?.dirty || buffer.version !== entry.version || app.bufferConflicts?.[id]) {
        clearTimeout(entry.timer); timers.delete(id);
      }
    }
    if (!preferences.autoSave || restoring) return;
    for (const buffer of bootstrap.buffers) {
      const id = buffer.buffer_id;
      if (!buffer.dirty || app.bufferConflicts?.[id] || timers.has(id) || saving.has(id) || attempted.get(id) === buffer.version) continue;
      const timer = setTimeout(() => {
        timers.delete(id);
        const current = getAppSnapshot();
        const candidate = current.bootstrap?.buffers.find((item) => item.buffer_id === id);
        if (stopped || current.bootstrap?.workspace.root !== root || !localEditorPreferences.getSnapshot().autoSave || !candidate?.dirty || candidate.version !== buffer.version || current.bufferConflicts?.[id]) return;
        saving.add(id); attempted.set(id, buffer.version);
        void editorStore.saveBuffer(id).finally(() => { saving.delete(id); sync(); });
      }, preferences.autoSaveDelay);
      timers.set(id, { version: buffer.version, timer });
    }
  };
  const unsubscribe = subscribeApp(sync);
  const unsubscribePreferences = localEditorPreferences.subscribe(() => {
    for (const entry of timers.values()) clearTimeout(entry.timer);
    timers.clear(); attempted.clear(); sync();
  });
  // Also captures cursor-only changes that do not create a new buffer snapshot.
  const interval = setInterval(() => { if (!restoring && !stopped) sync(); }, 500);
  const flush = () => {
    const bootstrap = getAppSnapshot().bootstrap;
    if (!restoring && bootstrap) persistSession(bootstrap);
  };
  window.addEventListener('pagehide', flush);
  window.addEventListener('beforeunload', flush);
  sync();
  return () => {
    stopped = true; generation++;
    for (const finish of pendingWaits) finish();
    unsubscribe(); unsubscribePreferences(); clearInterval(interval);
    window.removeEventListener('pagehide', flush);
    window.removeEventListener('beforeunload', flush);
    for (const entry of timers.values()) clearTimeout(entry.timer);
  };
}
