import { describe, expect, it, vi } from 'vitest';
import { markPendingEditorEdit, settlePendingEditorEdit } from './pending-edits';

const mocked = vi.hoisted(() => {
  const buffer = { buffer_id: 'close-pending-buffer', path: 'file.txt', language: 'plain', text: 'saved', version: 1, dirty: false, selection: null };
  const bootstrap = { workspace: { id: 'close-pending-workspace', root: 'root', display_name: 'root' }, buffers: [buffer], active_buffer_id: buffer.buffer_id, file_tree: { root: null } };
  return { buffer, bootstrap, snapshot: { bootstrap, bufferConflicts: {} }, close: vi.fn().mockResolvedValue(undefined) };
});

vi.mock('../../app/app-store', () => ({
  getAppSnapshot: () => mocked.snapshot,
  currentEnvelopeContext: () => ({ workspace_id: mocked.bootstrap.workspace.id, core_instance_id: 'core', session_id: null }),
  subscribeApp: () => () => undefined,
}));
vi.mock('../../bridge/bridge', () => ({
  closeBuffer: mocked.close,
  keepBufferEdits: vi.fn(), openFile: vi.fn(), reloadBufferFromDisk: vi.fn(),
  saveBuffer: vi.fn(), setActiveBuffer: vi.fn(),
}));
vi.mock('../toasts/toast-store', () => ({ toastStore: { pushLocal: vi.fn() } }));

import { editorStore } from './editor-store';

describe('editor close with pending IPC', () => {
  it('never dispatches close for pending changes or stale clean UI', () => {
    markPendingEditorEdit(mocked.buffer.buffer_id, mocked.bootstrap.workspace.id, mocked.buffer.path, 2);
    editorStore.closeBuffer(mocked.buffer.buffer_id, true);
    expect(mocked.close).not.toHaveBeenCalled();

    settlePendingEditorEdit(mocked.bootstrap.workspace.id, mocked.buffer.buffer_id, true);
    mocked.buffer.version = 2;
    mocked.buffer.dirty = true;
    editorStore.reset();
    editorStore.closeBuffer(mocked.buffer.buffer_id);
    expect(mocked.close).not.toHaveBeenCalled();
    editorStore.closeBuffer(mocked.buffer.buffer_id, true);
    expect(mocked.close).toHaveBeenCalledOnce();
  });
});
