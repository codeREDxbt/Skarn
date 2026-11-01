import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ExplorerContextMenu } from './ExplorerContextMenu';

describe('ExplorerContextMenu', () => {
  it('offers folder actions and dispatches the selected action', () => {
    const onAction = vi.fn();
    const onClose = vi.fn();
    render(<ExplorerContextMenu name="src" isDirectory isRoot={false} x={20} y={30} onAction={onAction} onClose={onClose} />);
    expect(screen.getAllByRole('menuitem')).toHaveLength(4);
    fireEvent.click(screen.getByRole('menuitem', { name: 'New File' }));
    expect(onAction).toHaveBeenCalledWith('new-file');
    expect(onClose).toHaveBeenCalled();
  });

  it('limits root actions and supports keyboard dismissal', () => {
    const onClose = vi.fn();
    render(<ExplorerContextMenu name="workspace" isDirectory isRoot x={20} y={30} onAction={vi.fn()} onClose={onClose} />);
    expect(screen.getAllByRole('menuitem')).toHaveLength(2);
    expect(screen.queryByRole('menuitem', { name: 'Delete' })).toBeNull();
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('offers only rename and delete for files', () => {
    render(<ExplorerContextMenu name="main.rs" isDirectory={false} isRoot={false} x={20} y={30} onAction={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getAllByRole('menuitem').map((item) => item.textContent)).toEqual(['Rename', 'Delete']);
  });
});
