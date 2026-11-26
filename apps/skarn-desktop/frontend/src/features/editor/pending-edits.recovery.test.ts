import { describe, expect, it } from 'vitest';
import type { BootstrapState } from '../../bridge/bridge';
import {
  attachEditorView,
  beginEditorClose,
  detachEditorView,
  finishEditorClose,
  hasPendingBufferEdits,
  isEditorClosing,
  isEditorPathClosing,
  markPendingEditorEdit,
  reconcileEditorCloses,
  reconcilePendingEditorEdits,
  reconcilePendingEditorEditText,
  registerPendingEditResync,
  rejectEditorClose,
  retryPendingEditorEdits,
  settlePendingEditorEdit,
} from './pending-edits';

function state(
  workspaceId: string,
  bufferId: string,
  version: number | null,
  path = 'src/file.ts',
): BootstrapState {
  return {
    workspace: { id: workspaceId },
    buffers:
      version === null
        ? []
        : [{ buffer_id: bufferId, path, version, language: 'plain', text: '', dirty: false, selection: null }],
  } as unknown as BootstrapState;
}

describe('pending editor edit recovery', () => {
  it('keeps a failed edit locked until the backend text is proven equal', () => {
    const workspaceId = 'recovery-reconcile-workspace';
    const bufferId = 'recovery-reconcile-buffer';
    markPendingEditorEdit(bufferId, workspaceId, 'src/a.ts', 2);
    settlePendingEditorEdit(workspaceId, bufferId, false);
    expect(hasPendingBufferEdits(state(workspaceId, bufferId, 1, 'src/a.ts'), bufferId)).toBe(true);

    // Another writer can advance the version without including the local text,
    // so version catch-up alone must not unlock a rejected batch.
    reconcilePendingEditorEdits(state(workspaceId, bufferId, 2, 'src/a.ts'));
    expect(hasPendingBufferEdits(state(workspaceId, bufferId, 2, 'src/a.ts'), bufferId)).toBe(true);

    // The pane proves the backend holds the same text: the edit landed and the
    // failed marker must not keep blocking save or close.
    reconcilePendingEditorEditText(workspaceId, bufferId, 'local text', 'other text');
    expect(hasPendingBufferEdits(state(workspaceId, bufferId, 2, 'src/a.ts'), bufferId)).toBe(true);
    reconcilePendingEditorEditText(workspaceId, bufferId, 'local text', 'local text');
    expect(hasPendingBufferEdits(state(workspaceId, bufferId, 2, 'src/a.ts'), bufferId)).toBe(false);
    expect(beginEditorClose(state(workspaceId, bufferId, 2, 'src/a.ts'), bufferId)).toBe(true);
    finishEditorClose(bufferId, true);
    reconcileEditorCloses(state(workspaceId, bufferId, null));
  });

  it('retries a failed edit through the pane resync and unlocks save', async () => {
    const workspaceId = 'recovery-retry-workspace';
    const bufferId = 'recovery-retry-buffer';
    const calls: string[] = [];
    const dispose = registerPendingEditResync(bufferId, async (id) => {
      calls.push(id);
      return 2;
    });
    try {
      markPendingEditorEdit(bufferId, workspaceId, 'src/b.ts', 2);
      settlePendingEditorEdit(workspaceId, bufferId, false);

      await expect(retryPendingEditorEdits(state(workspaceId, bufferId, 1), bufferId)).resolves.toBe(true);
      expect(calls).toEqual([bufferId]);
      expect(hasPendingBufferEdits(state(workspaceId, bufferId, 2), bufferId)).toBe(false);
    } finally {
      dispose();
    }
  });

  it('keeps the buffer locked when the resync fails', async () => {
    const workspaceId = 'recovery-failed-resync-workspace';
    const bufferId = 'recovery-failed-resync-buffer';
    const dispose = registerPendingEditResync(bufferId, async () => {
      throw new Error('offline');
    });
    try {
      markPendingEditorEdit(bufferId, workspaceId, 'src/c.ts', 2);
      settlePendingEditorEdit(workspaceId, bufferId, false);

      await expect(retryPendingEditorEdits(state(workspaceId, bufferId, 1), bufferId)).resolves.toBe(false);
      expect(hasPendingBufferEdits(state(workspaceId, bufferId, 1), bufferId)).toBe(true);
      expect(beginEditorClose(state(workspaceId, bufferId, 1), bufferId)).toBe(false);
    } finally {
      dispose();
    }
  });

  it('keeps protection for edits started while a retry is in flight', async () => {
    const workspaceId = 'recovery-concurrent-workspace';
    const bufferId = 'recovery-concurrent-buffer';
    let release!: (value: number | null) => void;
    const gate = new Promise<number | null>((resolve) => {
      release = resolve;
    });
    const dispose = registerPendingEditResync(bufferId, () => gate);
    try {
      markPendingEditorEdit(bufferId, workspaceId, 'src/d.ts', 2);
      settlePendingEditorEdit(workspaceId, bufferId, false);
      const retry = retryPendingEditorEdits(state(workspaceId, bufferId, 1), bufferId);

      // A keystroke lands while the resync is still running.
      markPendingEditorEdit(bufferId, workspaceId, 'src/d.ts', 3);
      release(2);

      await expect(retry).resolves.toBe(false);
      expect(hasPendingBufferEdits(state(workspaceId, bufferId, 2), bufferId)).toBe(true);

      settlePendingEditorEdit(workspaceId, bufferId, true);
      reconcilePendingEditorEdits(state(workspaceId, bufferId, 3));
      expect(hasPendingBufferEdits(state(workspaceId, bufferId, 3), bufferId)).toBe(false);
    } finally {
      dispose();
    }
  });
});

describe('rejected editor close recovery', () => {
  it('unlocks a definite close rejection that leaves the buffer open', () => {
    const workspaceId = 'recovery-close-open-workspace';
    const bufferId = 'recovery-close-open-buffer';
    expect(beginEditorClose(state(workspaceId, bufferId, 1, 'src/e.ts'), bufferId)).toBe(true);
    expect(isEditorPathClosing(workspaceId, 'src/e.ts')).toBe(true);

    rejectEditorClose(bufferId, 'still-open');
    expect(isEditorClosing(bufferId)).toBe(false);
    expect(isEditorPathClosing(workspaceId, 'src/e.ts')).toBe(false);
  });

  it('treats unknown_buffer as a completed close and releases after detach', () => {
    const workspaceId = 'recovery-close-gone-workspace';
    const bufferId = 'recovery-close-gone-buffer';
    attachEditorView(bufferId);
    expect(beginEditorClose(state(workspaceId, bufferId, 1, 'src/f.ts'), bufferId)).toBe(true);

    rejectEditorClose(bufferId, 'closed');
    expect(isEditorClosing(bufferId)).toBe(true);
    detachEditorView(bufferId);
    expect(isEditorClosing(bufferId)).toBe(false);
  });

  it('keeps an ambiguous close failure locked', () => {
    const workspaceId = 'recovery-close-ambiguous-workspace';
    const bufferId = 'recovery-close-ambiguous-buffer';
    expect(beginEditorClose(state(workspaceId, bufferId, 1, 'src/g.ts'), bufferId)).toBe(true);

    rejectEditorClose(bufferId, 'ambiguous');
    expect(isEditorClosing(bufferId)).toBe(true);
    expect(isEditorPathClosing(workspaceId, 'src/g.ts')).toBe(true);

    finishEditorClose(bufferId, true);
    reconcileEditorCloses(state(workspaceId, bufferId, null));
  });
});