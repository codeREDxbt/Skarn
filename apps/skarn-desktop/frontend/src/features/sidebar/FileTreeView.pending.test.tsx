import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { BootstrapState } from '../../bridge/bridge';
import { markPendingEditorEdit } from '../editor/pending-edits';

const mocked = vi.hoisted(() => {
  const bootstrap = {
    workspace: { id: 'pending-test-workspace', display_name: 'Workspace' },
    buffers: [], active_buffer_id: null,
    file_tree: { root: { path: 'root', name: 'Workspace', kind: 'directory', children: [
      { path: 'folder', name: 'folder', kind: 'directory', children: [] },
    ] } },
  };
  return { bootstrap, snapshot: { bootstrap }, deleteEntry: vi.fn(), renameEntry: vi.fn() };
});

vi.mock('../../app/app-store', () => ({
  getAppSnapshot: () => mocked.snapshot,
  subscribeApp: () => () => undefined,
  currentEnvelopeContext: () => null,
}));
vi.mock('../editor/editor-store', () => ({ editorStore: { getSnapshot: () => ({ tabs: [] }), openPath: vi.fn() } }));
vi.mock('../../bridge/explorer', () => ({
  explorerDeleteEntry: mocked.deleteEntry,
  explorerRenameEntry: mocked.renameEntry,
  explorerCreateEntry: vi.fn(),
}));

import { FileTreeView } from './FileTreeView';

describe('Explorer pending edits', () => {
  it('does not invoke deletion when a missing buffer still has an in-flight edit', async () => {
    markPendingEditorEdit('pending-file', mocked.bootstrap.workspace.id, 'folder/file.txt', 2);
    render(<FileTreeView bootstrap={mocked.bootstrap as unknown as BootstrapState} />);
    fireEvent.click(screen.getByRole('button', { name: 'Delete folder' }));
    fireEvent.click(screen.getByRole('button', { name: 'Permanently delete' }));
    expect((await screen.findByRole('alert')).textContent).toContain('unsynchronized');
    expect(mocked.deleteEntry).not.toHaveBeenCalled();
  });
});
