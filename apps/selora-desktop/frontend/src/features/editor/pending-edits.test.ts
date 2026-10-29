import { describe, expect, it } from 'vitest';
import type { BootstrapState } from '../../bridge/bridge';
import { attachEditorView, beginEditorClose, detachEditorView, finishEditorClose, hasPendingBufferEdits, hasPendingEditorEdits, isEditorClosing, markPendingEditorEdit, reconcileEditorCloses, settlePendingEditorEdit } from './pending-edits';

function bootstrap(id: string, version: number | null): BootstrapState {
  return { workspace: { id }, buffers: version === null ? [] : [{ buffer_id: 'buffer', path: 'folder/file', version }] } as unknown as BootstrapState;
}

describe('pending editor close barrier', () => {
  it('blocks close until IPC succeeds and its version is visible', () => {
    const old = bootstrap('pending-close-test', 1);
    const echoed = bootstrap('pending-close-test', 2);
    markPendingEditorEdit('buffer', old.workspace.id, 'folder/file', 2);
    expect(beginEditorClose(old, 'buffer')).toBe(false);
    expect(beginEditorClose(echoed, 'buffer')).toBe(false);
    settlePendingEditorEdit(old.workspace.id, 'buffer', true);
    expect(beginEditorClose(old, 'buffer')).toBe(false);
    expect(beginEditorClose(echoed, 'buffer')).toBe(true);
    expect(isEditorClosing('buffer')).toBe(true);
    finishEditorClose('buffer', true);
    reconcileEditorCloses(bootstrap('pending-close-test', null));
  });

  it('retains failed edits until the pane proves the backend text matches', () => {
    const state = bootstrap('failed-close-test', 1);
    markPendingEditorEdit('buffer', state.workspace.id, 'folder/file', 2);
    settlePendingEditorEdit(state.workspace.id, 'buffer', false);
    expect(hasPendingBufferEdits(bootstrap('failed-close-test', 1), 'buffer')).toBe(true);
    expect(hasPendingEditorEdits(bootstrap('failed-close-test', null), 'folder', true)).toBe(true);
    expect(beginEditorClose(state, 'buffer')).toBe(false);
    // A version catch-up alone is not proof: another writer can advance it
    // without including the rejected local text, so the marker stays.
    expect(hasPendingBufferEdits(bootstrap('failed-close-test', 2), 'buffer')).toBe(true);
  });

  it('keeps the editor locked until the close is acknowledged, absent, and detached', () => {
    const state = bootstrap('detach-close-test', 1);
    attachEditorView('buffer');
    expect(beginEditorClose(state, 'buffer')).toBe(true);
    reconcileEditorCloses(bootstrap('detach-close-test', null));
    finishEditorClose('buffer', true);
    expect(isEditorClosing('buffer')).toBe(true);
    detachEditorView('buffer');
    expect(isEditorClosing('buffer')).toBe(false);
  });

  it('keeps the editor locked when the close result is ambiguous', () => {
    const state = bootstrap('failed-transport-close-test', 1);
    expect(beginEditorClose(state, 'buffer')).toBe(true);
    finishEditorClose('buffer', false);
    reconcileEditorCloses(bootstrap('failed-transport-close-test', null));
    expect(isEditorClosing('buffer')).toBe(true);
  });
});
