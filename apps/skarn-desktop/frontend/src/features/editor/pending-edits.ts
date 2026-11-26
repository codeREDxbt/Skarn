import type { BootstrapState } from '../../bridge/bridge';

type Pending = {
  bufferId: string;
  workspaceId: string;
  path: string;
  version: number;
  inFlight: number;
  failed: boolean;
  retrying: boolean;
};
type Closing = { workspaceId: string; path: string; acknowledged: boolean; absent: boolean };
const pending = new Map<string, Pending>();
const closing = new Map<string, Closing>();
const views = new Map<string, number>();
const normalize = (path: string) => path.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
const pendingKey = (workspaceId: string, bufferId: string) => `${workspaceId}\0${bufferId}`;

/**
 * Re-publish one buffer's whole local text at the version the backend currently
 * reports. Registered by the live editor pane, because only it owns the local
 * document. Resolves with the backend version the text now carries, or null
 * when the backend already held the same text.
 */
export type PendingEditResync = (bufferId: string) => Promise<number | null>;
const resyncers = new Map<string, PendingEditResync>();

/** Register the pane's recovery path for one buffer; returns its disposer. */
export function registerPendingEditResync(bufferId: string, resync: PendingEditResync): () => void {
  resyncers.set(bufferId, resync);
  return () => {
    if (resyncers.get(bufferId) === resync) resyncers.delete(bufferId);
  };
}

/** Register synchronously before sending an editor change. */
export function markPendingEditorEdit(bufferId: string, workspaceId: string, path: string, version: number): void {
  const key = pendingKey(workspaceId, bufferId);
  const previous = pending.get(key);
  if (previous) {
    // Keep the identity stable: a retry in progress must still observe newer
    // in-flight edits instead of deleting a probe it no longer owns.
    previous.path = path;
    previous.version = Math.max(version, previous.version);
    previous.inFlight += 1;
    return;
  }
  pending.set(key, {
    bufferId, workspaceId, path, version, inFlight: 1, failed: false, retrying: false,
  });
}

/**
 * A failed edit stays blocked until the backend version proves it landed or
 * the pane re-publishes the local text: its local text has no safe backend copy
 * before that point.
 */
export function settlePendingEditorEdit(workspaceId: string, bufferId: string, success: boolean): void {
  const entry = pending.get(pendingKey(workspaceId, bufferId));
  if (!entry) return;
  entry.inFlight = Math.max(0, entry.inFlight - 1);
  if (!success) entry.failed = true;
}

/**
 * Drop pending markers the backend has caught up with. Call on every
 * bootstrap: an edit whose acknowledgement was lost still lands on the next
 * snapshot, so a failed marker must not outlive the version it predicted.
 */
export function reconcilePendingEditorEdits(bootstrap: BootstrapState | null): void {
  if (!bootstrap) return;
  for (const [key, entry] of pending) {
    if (entry.workspaceId && entry.workspaceId !== bootstrap.workspace.id) continue;
    if (entry.inFlight > 0) continue;
    // Version alone is not proof for a rejected batch: another writer can
    // advance it without including the local text. Failed entries wait for the
    // pane's text comparison or an explicit resync.
    if (entry.failed) continue;
    const buffer = bootstrap.buffers.find((candidate) => candidate.buffer_id === entry.bufferId);
    if (buffer && buffer.version >= entry.version) pending.delete(key);
  }
}

/**
 * Clear a pending marker only when the pane proves the backend holds the same
 * text. Used for entries whose acknowledgement failed: version equality alone
 * cannot rule out another writer having advanced the backend.
 */
export function reconcilePendingEditorEditText(
  workspaceId: string,
  bufferId: string,
  backendText: string,
  localText: string,
): void {
  const key = pendingKey(workspaceId, bufferId);
  const entry = pending.get(key);
  if (!entry || entry.inFlight > 0 || backendText !== localText) return;
  pending.delete(key);
}

/**
 * User-visible recovery for edits a transport failure left in doubt. The pane
 * re-publishes the local document at the version the backend reports, so no
 * local text is dropped. Resolves true only once the pending marker is gone;
 * any ambiguous state keeps the buffer locked.
 */
export async function retryPendingEditorEdits(bootstrap: BootstrapState, bufferId: string): Promise<boolean> {
  const key = pendingKey(bootstrap.workspace.id, bufferId);
  const entry = pending.get(key);
  if (!entry) return true;
  if (entry.inFlight > 0 || entry.retrying) return false;
  const buffer = bootstrap.buffers.find((candidate) => candidate.buffer_id === bufferId);
  if (!entry.failed && buffer && buffer.version >= entry.version) {
    pending.delete(key);
    return true;
  }
  const resync = resyncers.get(bufferId);
  if (!resync) return false;
  entry.retrying = true;
  let syncedVersion: number | null;
  try {
    syncedVersion = await resync(bufferId);
  } catch {
    // A bootstrap may have proven the edit landed while the resync ran.
    if (!pending.has(key)) return true;
    entry.retrying = false;
    entry.failed = true;
    return false;
  }
  entry.retrying = false;
  const current = pending.get(key);
  if (!current) return true;
  current.failed = false;
  if (current.inFlight === 0 && (syncedVersion === null || current.version <= syncedVersion)) {
    pending.delete(key);
    return true;
  }
  return false;
}

export function hasPendingBufferEdits(bootstrap: BootstrapState, bufferId: string): boolean {
  if (pending.has(pendingKey('', bufferId))) return true;
  const key = pendingKey(bootstrap.workspace.id, bufferId);
  const entry = pending.get(key);
  if (!entry) return false;
  const buffer = bootstrap.workspace.id === entry.workspaceId
    ? bootstrap.buffers.find((candidate) => candidate.buffer_id === bufferId) : undefined;
  // A failed batch stays blocked until the pane proves the backend holds the
  // same text or a resync succeeds: its version can advance for someone else.
  if (!entry.failed && entry.inFlight === 0 && buffer && buffer.version >= entry.version) {
    pending.delete(key);
    return false;
  }
  return true;
}

export function hasPendingEditorEdits(bootstrap: BootstrapState, path: string, directory: boolean): boolean {
  const target = normalize(path);
  for (const entry of pending.values()) {
    if (entry.workspaceId && entry.workspaceId !== bootstrap.workspace.id) continue;
    const candidate = normalize(entry.path);
    if ((candidate === target || (directory && candidate.startsWith(target + '/'))) && (!entry.workspaceId || hasPendingBufferEdits(bootstrap, entry.bufferId))) return true;
  }
  return false;
}

/** Lock the editor before dispatching close; no document changes may start afterwards. */
export function beginEditorClose(bootstrap: BootstrapState, bufferId: string): boolean {
  const buffer = bootstrap.buffers.find((candidate) => candidate.buffer_id === bufferId);
  if (!buffer || closing.has(bufferId) || hasPendingBufferEdits(bootstrap, bufferId)) return false;
  closing.set(bufferId, { workspaceId: bootstrap.workspace.id, path: buffer.path, acknowledged: false, absent: false });
  return true;
}

export function isEditorClosing(bufferId: string): boolean { return closing.has(bufferId); }

export function isEditorPathClosing(workspaceId: string, path: string): boolean {
  return [...closing.values()].some((entry) => entry.workspaceId === workspaceId && normalize(entry.path) === normalize(path));
}

function releaseClosed(bufferId: string): void {
  const entry = closing.get(bufferId);
  if (entry?.acknowledged && entry.absent && !views.has(bufferId)) closing.delete(bufferId);
}

export function finishEditorClose(bufferId: string, success: boolean): void {
  // A rejected transport does not prove that Rust did not already close the buffer.
  if (!success) return;
  const entry = closing.get(bufferId);
  if (entry) entry.acknowledged = true;
  releaseClosed(bufferId);
}

export type EditorCloseRejection = 'ambiguous' | 'still-open' | 'closed';

/**
 * Reconcile a rejected close against the backend's own answer. A protocol
 * error is definite: the backend either still holds the buffer
 * (`still-open`) or has already forgotten it (`closed`). A transport failure
 * stays `ambiguous` and keeps the lock.
 */
export function rejectEditorClose(bufferId: string, rejection: EditorCloseRejection): void {
  const entry = closing.get(bufferId);
  if (!entry || rejection === 'ambiguous') return;
  if (rejection === 'still-open') {
    // The backend refused the close, so the buffer and its path stay usable.
    closing.delete(bufferId);
    return;
  }
  entry.acknowledged = true;
  entry.absent = true;
  releaseClosed(bufferId);
}

export function reconcileEditorCloses(bootstrap: BootstrapState | null): void {
  for (const [id, entry] of closing) {
    if (!bootstrap || bootstrap.workspace.id !== entry.workspaceId || !bootstrap.buffers.some((buffer) => buffer.buffer_id === id)) entry.absent = true;
    releaseClosed(id);
  }
}

export function attachEditorView(bufferId: string): void {
  views.set(bufferId, (views.get(bufferId) ?? 0) + 1);
}

export function detachEditorView(bufferId: string): void {
  const count = views.get(bufferId) ?? 0;
  if (count <= 1) views.delete(bufferId); else views.set(bufferId, count - 1);
  releaseClosed(bufferId);
}
