import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { ExplorerOperation } from './ExplorerDialog';

type Action = ExplorerOperation['action'];

export function ExplorerContextMenu({ name, isDirectory, isRoot, x, y, onAction, onClose }: {
  name: string;
  isDirectory: boolean;
  isRoot: boolean;
  x: number;
  y: number;
  onAction: (action: Action) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    setPosition({
      left: Math.max(8, Math.min(x, window.innerWidth - rect.width - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - rect.height - 8)),
    });
    menu.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
  }, [x, y]);

  useEffect(() => {
    const dismiss = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    window.addEventListener('mousedown', dismiss);
    return () => window.removeEventListener('mousedown', dismiss);
  }, [onClose]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const items = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
    if (!items.length) return;
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
      : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[next]?.focus();
  };

  const items: Array<{ action: Action; label: string }> = [];
  if (isDirectory) items.push({ action: 'new-file', label: 'New File' }, { action: 'new-folder', label: 'New Folder' });
  if (!isRoot) items.push({ action: 'rename', label: 'Rename' }, { action: 'delete', label: 'Delete' });

  return (
    <div ref={ref} className="tree-context-menu" role="menu" aria-label={`Actions for ${name}`}
      style={position} onKeyDown={handleKeyDown} onContextMenu={(event) => event.preventDefault()}>
      {items.map((item, index) => (
        <span key={item.action} style={{ display: 'contents' }}>
          {index === 2 && isDirectory ? <div className="context-menu-divider" role="separator" /> : null}
          <button type="button" role="menuitem" className={`context-menu-item${item.action === 'delete' ? ' is-danger' : ''}`}
            onClick={() => { onAction(item.action); onClose(); }}>
            {item.label}
          </button>
        </span>
      ))}
    </div>
  );
}
