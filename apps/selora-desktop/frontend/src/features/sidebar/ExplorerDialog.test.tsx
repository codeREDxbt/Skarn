import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ExplorerDialog } from './ExplorerDialog';

describe('ExplorerDialog', () => {
  it('requires explicit confirmation for permanent recursive deletion', async () => {
    const submit = vi.fn().mockResolvedValue(undefined);
    const close = vi.fn();
    render(<ExplorerDialog operation={{ action: 'delete', path: 'folder', name: 'folder', isDirectory: true, workspaceId: 'workspace' }} onSubmit={submit} onClose={close} />);
    expect(screen.getByText(/recursively deletes/).textContent).toContain('cannot be undone');
    expect(submit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Permanently delete' }));
    await waitFor(() => expect(submit).toHaveBeenCalledOnce());
    await waitFor(() => expect(close).toHaveBeenCalledOnce());
  });

  it('shows mutation errors without closing the dialog', async () => {
    const close = vi.fn();
    render(<ExplorerDialog operation={{ action: 'rename', path: 'file', name: 'file', isDirectory: false, workspaceId: 'workspace' }} onSubmit={vi.fn().mockRejectedValue('Save unsaved files first.')} onClose={close} />);
    fireEvent.click(screen.getByRole('button', { name: 'Rename' }));
    expect((await screen.findByRole('alert')).textContent).toContain('Save unsaved files first.');
    expect(close).not.toHaveBeenCalled();
  });
});
