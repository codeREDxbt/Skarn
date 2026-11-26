// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => {
  const listeners = new Set<() => void>();
  const preferences = { autoSave: false, autoSaveDelay: 500, restoreSession: true };
  const app: any = { phase: 'ready', bootstrap: null, bufferConflicts: {} };
  const store: any = { tabs: [], activeBuffer: null, pinnedPaths: [], previewPath: null };
  return {
    listeners,
    preferences,
    app,
    store,
    openFile: vi.fn(),
    saveBuffer: vi.fn(),
    setActive: vi.fn(),
    restoreTabAffordances: vi.fn(),
  };
});
vi.mock('../../app/app-store', () => ({
  getAppSnapshot: () => mocks.app,
  currentEnvelopeContext: () => ({ workspace_id: mocks.app.bootstrap?.workspace.id }),
  subscribeApp: (listener: () => void) => { mocks.listeners.add(listener); return () => mocks.listeners.delete(listener); },
}));
vi.mock('../../bridge/bridge', () => ({ openFile: mocks.openFile }));
vi.mock('./local-preferences', () => ({ localEditorPreferences: {
  getSnapshot: () => mocks.preferences,
  subscribe: () => () => {},
} }));
vi.mock('./editor-store', () => ({ editorStore: {
  getSnapshot: () => mocks.store,
  saveBuffer: mocks.saveBuffer,
  setActive: mocks.setActive,
  restoreTabAffordances: mocks.restoreTabAffordances,
} }));

import { startEditorPersistence, savedEditorSelection } from './editor-persistence';

describe('editor persistence', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    mocks.listeners.clear();
    mocks.openFile.mockReset();
    mocks.saveBuffer.mockReset();
    mocks.setActive.mockReset();
    mocks.restoreTabAffordances.mockReset();
    mocks.preferences.autoSave = false;
    mocks.preferences.restoreSession = true;
    mocks.store.tabs = [];
    mocks.store.activeBuffer = null;
    mocks.store.pinnedPaths = [];
    mocks.store.previewPath = null;
    mocks.app.bootstrap = { workspace: { root: 'root-a', id: 'a' }, buffers: [], active_buffer_id: null };
  });
  afterEach(() => vi.useRealTimers());

  it('does not autosave by default or retain timers after cleanup', () => {
    mocks.app.bootstrap.buffers = [{ buffer_id: '1', path: 'a.txt', dirty: true, version: 1, selection: null }];
    const stop = startEditorPersistence();
    vi.advanceTimersByTime(2000);
    expect(mocks.saveBuffer).not.toHaveBeenCalled();
    stop();
    expect(mocks.listeners.size).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
    expect(localStorage.getItem('skarn:editor-session:root-a')).not.toContain('text');
  });

  it('keeps case-sensitive workspace roots separate', () => {
    mocks.app.bootstrap.workspace.root = '/work/Project';
    mocks.app.bootstrap.buffers = [{ buffer_id: '1', path: 'upper.txt', dirty: false, version: 1, selection: null }];
    const stop = startEditorPersistence();
    mocks.app.bootstrap = { workspace: { root: '/work/project', id: 'b' }, buffers: [], active_buffer_id: null };
    for (const listener of mocks.listeners) listener();
    expect(JSON.parse(localStorage.getItem('skarn:editor-session:/work/Project')!).files[0].path).toBe('upper.txt');
    expect(JSON.parse(localStorage.getItem('skarn:editor-session:/work/project')!).files).toEqual([]);
    stop();
  });

  it('restores only clean tabs and keeps unsaved work out of the session', () => {
    mocks.app.bootstrap.buffers = [
      { buffer_id: '1', path: 'clean.txt', dirty: false, version: 1, selection: null },
      { buffer_id: '2', path: 'dirty.txt', dirty: true, version: 2, selection: null },
    ];
    mocks.app.bootstrap.active_buffer_id = '2';
    mocks.store.activeBuffer = { buffer_id: '2', path: 'dirty.txt', dirty: true };
    mocks.store.pinnedPaths = ['clean.txt', 'dirty.txt'];
    mocks.store.previewPath = 'dirty.txt';
    const stop = startEditorPersistence();
    const session = JSON.parse(localStorage.getItem('skarn:editor-session:root-a')!);
    expect(session.files.map((file: { path: string }) => file.path)).toEqual(['clean.txt']);
    expect(session.activePath).toBeNull();
    expect(session.pinnedPaths).toEqual(['clean.txt']);
    expect(session.previewPath).toBeNull();
    stop();
  });

  it('persists the active tab, pins, and the preview path', () => {
    mocks.app.bootstrap.buffers = [
      { buffer_id: '1', path: 'a.txt', dirty: false, version: 1, selection: null },
      { buffer_id: '2', path: 'b.txt', dirty: false, version: 1, selection: null },
    ];
    mocks.app.bootstrap.active_buffer_id = '2';
    mocks.store.activeBuffer = { buffer_id: '2', path: 'b.txt', dirty: false };
    mocks.store.pinnedPaths = ['a.txt'];
    mocks.store.previewPath = 'b.txt';
    const stop = startEditorPersistence();
    const session = JSON.parse(localStorage.getItem('skarn:editor-session:root-a')!);
    expect(session.activePath).toBe('b.txt');
    expect(session.pinnedPaths).toEqual(['a.txt']);
    expect(session.previewPath).toBe('b.txt');
    stop();
  });

  it('reopens saved tabs and applies pins, preview, and the active tab', async () => {
    localStorage.setItem('skarn:editor-session:root-a', JSON.stringify({
      files: [{ path: 'a.txt', selection: null }],
      activePath: 'a.txt',
      pinnedPaths: ['a.txt'],
      previewPath: 'a.txt',
    }));
    mocks.openFile.mockResolvedValue(undefined);
    mocks.store.tabs = [{ id: 'buffer-a', path: 'a.txt' }];
    const stop = startEditorPersistence();
    await vi.advanceTimersByTimeAsync(2100);
    expect(mocks.openFile).toHaveBeenCalledWith('a.txt', expect.anything());
    expect(mocks.restoreTabAffordances).toHaveBeenCalledWith(['a.txt'], 'a.txt');
    expect(mocks.setActive).toHaveBeenCalledWith('buffer-a');
    stop();
  });

  it('restores saved cursors without touching the workspace restore path', async () => {
    const selection = {
      anchor: { line: 3, utf16_column: 2 },
      active: { line: 3, utf16_column: 2 },
      version: 1,
    };
    localStorage.setItem('skarn:editor-session:root-a', JSON.stringify({
      files: [{ path: 'a.txt', selection }],
      activePath: 'a.txt',
    }));
    mocks.openFile.mockResolvedValue(undefined);
    const stop = startEditorPersistence();
    expect(savedEditorSelection('a.txt')).toEqual(selection);
    await vi.advanceTimersByTimeAsync(2100);
    stop();
  });

  it('stops restoring the old workspace after a switch', async () => {
    localStorage.setItem('skarn:editor-session:root-a', JSON.stringify({ files: [{ path: 'a.txt' }, { path: 'b.txt' }], activePath: null }));
    let complete!: () => void;
    mocks.openFile.mockImplementation(() => new Promise<void>((resolve) => { complete = resolve; }));
    const stop = startEditorPersistence();
    expect(mocks.openFile).toHaveBeenCalledTimes(1);
    mocks.app.bootstrap = { workspace: { root: 'root-b', id: 'b' }, buffers: [], active_buffer_id: null };
    for (const listener of mocks.listeners) listener();
    complete();
    await vi.advanceTimersByTimeAsync(2100);
    expect(mocks.openFile).toHaveBeenCalledTimes(1);
    stop();
    expect(mocks.listeners.size).toBe(0);
  });
});