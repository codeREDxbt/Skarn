import { useEffect, useRef, useState } from 'react';
import './explorer-dialog.css';

export type ExplorerOperation = {
  action: 'new-file' | 'new-folder' | 'rename' | 'delete';
  path: string;
  name: string;
  isDirectory: boolean;
  workspaceId: string;
};

export function ExplorerDialog({ operation, onSubmit, onClose }: {
  operation: ExplorerOperation;
  onSubmit: (name: string) => Promise<void>;
  onClose: () => void;
}) {
  const [name, setName] = useState(operation.action === 'rename' ? operation.name : '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const formRef = useRef<HTMLFormElement>(null);
  const deleting = operation.action === 'delete';
  const title = deleting ? `Delete ${operation.name}` : operation.action === 'rename'
    ? `Rename ${operation.name}` : operation.action === 'new-file' ? 'New File' : 'New Folder';

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const input = formRef.current?.querySelector('input');
    (input ?? formRef.current?.querySelector('button'))?.focus();
    input?.select();
    return () => previous?.focus();
  }, []);

  return (
    <div className="explorer-dialog-backdrop">
      <form ref={formRef} className="explorer-dialog" role="dialog" aria-modal="true"
        aria-labelledby="explorer-dialog-title" aria-describedby={deleting ? 'explorer-delete-warning' : undefined}
        aria-busy={busy}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            if (!busy) onClose();
          }
          if (event.key === 'Tab') {
            const controls = Array.from(formRef.current?.querySelectorAll<HTMLElement>('input:not(:disabled), button:not(:disabled)') ?? []);
            const first = controls[0];
            const last = controls[controls.length - 1];
            if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
            else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
          }
        }}
        onSubmit={async (event) => {
          event.preventDefault();
          if (busy) return;
          if (!deleting && (!name.trim() || name === '.' || name === '..' || /[\\/\u0000-\u001f]/.test(name))) {
            setError('Enter a file or folder name without path separators, ".", or "..".');
            return;
          }
          setBusy(true);
          setError(null);
          try { await onSubmit(name); onClose(); }
          catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
          finally { setBusy(false); }
        }}>
        <h3 id="explorer-dialog-title">{title}</h3>
        {deleting ? (
          <p id="explorer-delete-warning">Permanently delete {operation.name}?
            {operation.isDirectory ? ' This recursively deletes the folder and all files and subfolders inside it.' : ''}
            {' This cannot be undone.'}</p>
        ) : (
          <label>Name<input value={name} disabled={busy} aria-invalid={Boolean(error)}
            aria-describedby={error ? 'explorer-dialog-error' : undefined}
            onChange={(event) => { setName(event.target.value); setError(null); }} /></label>
        )}
        {error ? <p id="explorer-dialog-error" role="alert">{error}</p> : null}
        {busy ? <p role="status">Working…</p> : null}
        <div className="explorer-dialog-actions">
          <button type="button" disabled={busy} onClick={onClose}>Cancel</button>
          <button type="submit" disabled={busy}>{deleting ? 'Permanently delete' : operation.action === 'rename' ? 'Rename' : 'Create'}</button>
        </div>
      </form>
    </div>
  );
}
