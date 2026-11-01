import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const mocked = vi.hoisted(() => ({
  closeBuffer: vi.fn(), closeMany: vi.fn(() => ({ closed: 1, kept: 0 })), saveBuffer: vi.fn(async () => true),
  snapshot: { tabs: [{ id: 'a', filename: 'a.ts', path: 'a.ts', isDirty: true }, { id: 'b', filename: 'b.ts', path: 'b.ts', isDirty: false }], activeBufferId: 'b', workspaceRoot: '/workspace' },
}));
vi.mock('../editor/editor-store', () => ({
  currentEditorContext: () => null,
  editorStore: { subscribe: () => () => undefined, getSnapshot: () => mocked.snapshot, closeBuffer: mocked.closeBuffer, closeMany: mocked.closeMany, saveBuffer: mocked.saveBuffer, setActive: vi.fn() },
}));
vi.mock('./FileTreeView', () => ({ FileTypeIcon: () => null }));
import { OpenEditors } from './OpenEditors';

describe('OpenEditors', () => {
  it('targets the right-clicked editor and protects dirty files on close', () => {
    render(<OpenEditors />);
    fireEvent.contextMenu(screen.getByRole('button', { name: /a.ts/ }), { clientX: 40, clientY: 50 });
    expect(screen.getByRole('menu', { name: 'Actions for a.ts' })).toBeTruthy();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Close' }));
    expect(mocked.closeBuffer).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog', { name: 'Unsaved changes in a.ts' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    expect(mocked.closeBuffer).toHaveBeenCalledWith('a', true);
  });

  it('closes other editors using the shared safe bulk-close operation', () => {
    render(<OpenEditors />);
    fireEvent.contextMenu(screen.getByRole('button', { name: /a.ts/ }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Close Others' }));
    expect(mocked.closeMany).toHaveBeenCalledWith(['b']);
  });
});
