// Preview tabs and pinned-tab persistence across bootstrap replays.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const snapshot: any = { phase: 'ready', bootstrap: null, bufferConflicts: {} };
  return {
    listeners,
    snapshot,
    openFile: vi.fn(),
    closeBuffer: vi.fn(),
    saveBuffer: vi.fn(),
    setActiveBuffer: vi.fn(),
    toast: vi.fn(),
  };
});

vi.mock('../../app/app-store', () => ({
  // The real store returns a fresh snapshot object per publish; mirror that so
  // the editor store notices changes in this harness.
  getAppSnapshot: () => ({ ...mocks.snapshot }),
  currentEnvelopeContext: () => ({ workspace_id: 'workspace', core_instance_id: 'core', session_id: null }),
  subscribeApp: (listener: () => void) => {
    mocks.listeners.add(listener);
    return () => mocks.listeners.delete(listener);
  },
}));
vi.mock('../../bridge/bridge', () => ({
  openFile: mocks.openFile,
  closeBuffer: mocks.closeBuffer,
  saveBuffer: mocks.saveBuffer,
  setActiveBuffer: mocks.setActiveBuffer,
  reloadBufferFromDisk: vi.fn(),
  keepBufferEdits: vi.fn(),
}));
vi.mock('../toasts/toast-store', () => ({ toastStore: { pushLocal: mocks.toast } }));

import { editorStore } from './editor-store';

let nextId = 1;

function buffer(path: string, overrides: Record<string, unknown> = {}) {
  return {
    buffer_id: `buffer-${nextId++}`,
    path,
    language: 'plain',
    text: '',
    version: 1,
    dirty: false,
    selection: null,
    ...overrides,
  };
}

function bootstrap(buffers: any[] = [], active: string | null = null) {
  return {
    protocol_version: 1,
    core_instance_id: 'core',
    snapshot_sequence: 1,
    workspace: { id: 'workspace', core_instance_id: 'core', root: '/workspace', display_name: 'workspace' },
    buffers,
    active_buffer_id: active,
    file_tree: { root: null },
    models: [],
    active_model_id: null,
    connection: { state: 'connected' },
    chat: { sessions: [], messages: [], active_session_id: null, active_request_id: null, active_turn_id: null },
    pending_approval: null,
  };
}

function emit(): void {
  for (const listener of mocks.listeners) listener();
}

function replaceBuffers(buffers: any[], active: string | null = null): void {
  mocks.snapshot.bootstrap = bootstrap(buffers, active);
  emit();
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.snapshot.bootstrap = bootstrap();
  mocks.openFile.mockImplementation(async (path: string) => {
    const existing = mocks.snapshot.bootstrap.buffers.find((item: any) => item.path === path);
    if (!existing) {
      const opened = buffer(path);
      mocks.snapshot.bootstrap.buffers.push(opened);
      mocks.snapshot.bootstrap.active_buffer_id = opened.buffer_id;
    }
    emit();
  });
  mocks.closeBuffer.mockImplementation(async (id: string) => {
    mocks.snapshot.bootstrap.buffers = mocks.snapshot.bootstrap.buffers.filter(
      (item: any) => item.buffer_id !== id,
    );
    emit();
  });
  editorStore.reset();
});

describe('editor preview tabs', () => {
  it('opens the first single-click file as a preview tab', async () => {
    await editorStore.openPath('/workspace/a.txt');
    const state = editorStore.getSnapshot();
    expect(state.tabs).toHaveLength(1);
    expect(state.tabs[0]).toMatchObject({ path: '/workspace/a.txt', isPreview: true });
    expect(state.previewPath).toBe('/workspace/a.txt');
  });

  it('replaces the preview tab when another file is single-clicked', async () => {
    await editorStore.openPath('/workspace/a.txt');
    const firstId = editorStore.getSnapshot().tabs[0].id;
    await editorStore.openPath('/workspace/b.txt');

    expect(mocks.closeBuffer).toHaveBeenCalledWith(firstId, expect.anything());
    const state = editorStore.getSnapshot();
    expect(state.previewPath).toBe('/workspace/b.txt');
    expect(state.tabs.map((tab) => tab.path)).toEqual(['/workspace/b.txt']);
    expect(state.tabs[0].isPreview).toBe(true);
  });

  it('opens permanent tabs when preview is disabled', async () => {
    await editorStore.openPath('/workspace/a.txt', { preview: false });
    const state = editorStore.getSnapshot();
    expect(state.previewPath).toBeNull();
    expect(state.tabs[0].isPreview).toBe(false);
  });

  it('promotes an existing preview when reopened permanently', async () => {
    await editorStore.openPath('/workspace/a.txt');
    await editorStore.openPath('/workspace/a.txt', { preview: false });
    const state = editorStore.getSnapshot();
    expect(state.previewPath).toBeNull();
    expect(state.tabs[0].isPreview).toBe(false);
  });

  it('stops treating a tab as preview once it is edited', async () => {
    await editorStore.openPath('/workspace/a.txt');
    const current = editorStore.getSnapshot().tabs[0];
    replaceBuffers([{ ...current, buffer_id: current.id, dirty: true }], current.id);

    const state = editorStore.getSnapshot();
    expect(state.previewPath).toBeNull();
    expect(state.tabs[0].isPreview).toBe(false);
  });

  it('pinning a preview makes it permanent', async () => {
    await editorStore.openPath('/workspace/a.txt');
    const id = editorStore.getSnapshot().tabs[0].id;
    editorStore.togglePinned(id);

    const state = editorStore.getSnapshot();
    expect(state.previewPath).toBeNull();
    expect(state.pinnedIds).toEqual([id]);
    expect(state.pinnedPaths).toEqual(['/workspace/a.txt']);
    expect(state.tabs[0].isPreview).toBe(false);
  });

  it('keeps pins by path when bootstrap reassigns buffer ids', async () => {
    await editorStore.openPath('/workspace/a.txt', { preview: false });
    const firstId = editorStore.getSnapshot().tabs[0].id;
    editorStore.togglePinned(firstId);

    const reopened = buffer('/workspace/a.txt');
    replaceBuffers([reopened], reopened.buffer_id);

    const state = editorStore.getSnapshot();
    expect(state.tabs[0].id).toBe(reopened.buffer_id);
    expect(state.pinnedIds).toEqual([reopened.buffer_id]);
    expect(state.pinnedPaths).toEqual(['/workspace/a.txt']);
  });

  it('restores pinned paths and a preview path from persistence', async () => {
    const a = buffer('/workspace/a.txt');
    const b = buffer('/workspace/b.txt');
    replaceBuffers([a, b], a.buffer_id);

    editorStore.restoreTabAffordances(['/workspace/a.txt'], '/workspace/b.txt');
    const state = editorStore.getSnapshot();
    expect(state.pinnedIds).toEqual([a.buffer_id]);
    expect(state.pinnedPaths).toEqual(['/workspace/a.txt']);
    expect(state.previewPath).toBe('/workspace/b.txt');
    expect(state.tabs.find((tab) => tab.id === b.buffer_id)?.isPreview).toBe(true);
  });
});