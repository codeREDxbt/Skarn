import { describe, expect, it, vi } from 'vitest';

const mocked = vi.hoisted(() => {
  const make = (bufferId: string, path: string) => ({
    buffer_id: bufferId,
    path,
    language: 'plain',
    text: 'saved',
    version: 1,
    dirty: false,
    selection: null,
  });
  const buffers = [
    make('reject-open-buffer', 'reject-open.txt'),
    make('reject-unknown-buffer', 'reject-unknown.txt'),
    make('reject-ambiguous-buffer', 'reject-ambiguous.txt'),
    make('save-retry-buffer', 'save-retry.txt'),
  ];
  const bootstrap = {
    workspace: { id: 'reject-workspace', root: 'root', display_name: 'root' },
    buffers,
    active_buffer_id: buffers[0].buffer_id,
    file_tree: { root: null },
  };
  return {
    buffers,
    bootstrap,
    snapshot: { bootstrap, bufferConflicts: {} },
    close: vi.fn(),
    save: vi.fn().mockResolvedValue(undefined),
  };
});

vi.mock('../../app/app-store', () => ({
  getAppSnapshot: () => mocked.snapshot,
  currentEnvelopeContext: () => ({
    workspace_id: mocked.bootstrap.workspace.id,
    core_instance_id: 'core',
    session_id: null,
  }),
  subscribeApp: () => () => undefined,
}));
vi.mock('../../bridge/bridge', () => ({
  closeBuffer: mocked.close,
  keepBufferEdits: vi.fn(),
  openFile: vi.fn(),
  reloadBufferFromDisk: vi.fn(),
  saveBuffer: mocked.save,
  setActiveBuffer: vi.fn(),
}));
vi.mock('../toasts/toast-store', () => ({ toastStore: { pushLocal: vi.fn() } }));

import { editorStore } from './editor-store';
import {
  isEditorClosing,
  isEditorPathClosing,
  markPendingEditorEdit,
  registerPendingEditResync,
  settlePendingEditorEdit,
} from './pending-edits';

describe('editor close rejection recovery', () => {
  it('unlocks the editor after a definite backend rejection keeps the buffer open', async () => {
    mocked.close.mockRejectedValueOnce({ code: 'workspace_busy', message: 'workspace busy', retryable: true });
    editorStore.closeBuffer('reject-open-buffer');
    expect(isEditorClosing('reject-open-buffer')).toBe(true);

    await vi.waitFor(() => expect(isEditorClosing('reject-open-buffer')).toBe(false));
    expect(isEditorPathClosing('reject-workspace', 'reject-open.txt')).toBe(false);
  });

  it('treats unknown_buffer as a completed close', async () => {
    mocked.close.mockRejectedValueOnce({ code: 'unknown_buffer', message: 'buffer is not open', retryable: false });
    editorStore.closeBuffer('reject-unknown-buffer');

    await vi.waitFor(() => expect(isEditorClosing('reject-unknown-buffer')).toBe(false));
  });

  it('keeps the editor and path locked when the transport result is ambiguous', async () => {
    mocked.close.mockRejectedValueOnce(new Error('transport closed'));
    editorStore.closeBuffer('reject-ambiguous-buffer');

    await vi.waitFor(() => expect(mocked.close).toHaveBeenCalledTimes(3));
    await Promise.resolve();
    expect(isEditorClosing('reject-ambiguous-buffer')).toBe(true);
    expect(isEditorPathClosing('reject-workspace', 'reject-ambiguous.txt')).toBe(true);
  });

  it('recovers a failed edit before saving instead of blocking forever', async () => {
    const dispose = registerPendingEditResync('save-retry-buffer', async () => 2);
    try {
      markPendingEditorEdit('save-retry-buffer', 'reject-workspace', 'save-retry.txt', 2);
      settlePendingEditorEdit('reject-workspace', 'save-retry-buffer', false);

      await expect(editorStore.saveBuffer('save-retry-buffer')).resolves.toBe(true);
      expect(mocked.save).toHaveBeenCalledOnce();
      expect(isEditorClosing('save-retry-buffer')).toBe(false);
    } finally {
      dispose();
    }
  });
});