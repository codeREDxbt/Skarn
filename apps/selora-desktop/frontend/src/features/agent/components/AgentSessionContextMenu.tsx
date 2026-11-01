// Selora IDE — agent session tab context menu.
//
// Right-clicking a session tab opens this fixed-position menu anchored at the
// pointer. Items are target-scoped: they apply to the clicked session, not the
// active one. Delete asks for confirmation because it removes the durable
// transcript on disk and cannot be undone.
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { AgentSession } from '../types';
import { currentEnvelopeContext } from '../../../app/app-store';
import { openAgentsWindow } from '../../../bridge/bridge';

export type AgentSessionContextMenuProps = {
  session: Pick<AgentSession, 'id' | 'title'>;
  /** Viewport coordinates of the right-click. */
  x: number;
  y: number;
  isPinned: boolean;
  /** The backend refuses a delete while the session's turn is running. */
  canDelete: boolean;
  onTogglePin: () => void;
  onDelete: () => void | Promise<void>;
  onClose: () => void;
  onOpenInAgents?: (sessionId: string) => void | Promise<void>;
  onRename?: () => void | Promise<void>;
  onCopy?: () => void | Promise<void>;
  onShowInMain?: () => void | Promise<void>;
};

const MENU_MARGIN = 8;

export function AgentSessionContextMenu({
  session,
  x,
  y,
  isPinned,
  canDelete,
  onTogglePin,
  onDelete,
  onClose,
  onOpenInAgents,
  onRename,
  onCopy,
  onShowInMain,
}: AgentSessionContextMenuProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ top: y, left: x });
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const openInAgents = async () => {
    try {
      if (onOpenInAgents) await onOpenInAgents(session.id);
      else {
        const context = currentEnvelopeContext();
        if (!context) throw new Error('Open a workspace before opening an Agents window.');
        await openAgentsWindow({ sessionId: session.id, workspaceId: context.workspace_id });
      }
      onClose();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    }
  };

  // Measure after paint so the menu never hangs off the viewport edge.
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const rect = root.getBoundingClientRect();
    const viewportWidth = window.innerWidth || 1200;
    const viewportHeight = window.innerHeight || 800;
    setPosition((current) => {
      const left = Math.max(
        MENU_MARGIN,
        Math.min(current.left, viewportWidth - rect.width - MENU_MARGIN),
      );
      const top = Math.max(
        MENU_MARGIN,
        Math.min(current.top, viewportHeight - rect.height - MENU_MARGIN),
      );
      return left === current.left && top === current.top ? current : { left, top };
    });
  }, []);

  useEffect(() => {
    rootRef.current
      ?.querySelector<HTMLButtonElement>('[role="menuitem"]:not([disabled])')
      ?.focus();
  }, [confirmingDelete]);

  useEffect(() => {
    const handlePointer = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target || rootRef.current?.contains(target)) return;
      onClose();
    };
    window.addEventListener('mousedown', handlePointer);
    return () => window.removeEventListener('mousedown', handlePointer);
  }, [onClose]);

  const moveFocus = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      if (confirmingDelete) {
        setConfirmingDelete(false);
        return;
      }
      onClose();
      return;
    }
    if (
      event.key !== 'ArrowDown' &&
      event.key !== 'ArrowUp' &&
      event.key !== 'Home' &&
      event.key !== 'End'
    ) {
      return;
    }
    event.preventDefault();
    const items = Array.from(
      rootRef.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not([disabled])',
      ) ?? [],
    );
    if (items.length === 0) return;
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    let next = 0;
    if (event.key === 'End') next = items.length - 1;
    else if (event.key !== 'Home') {
      const delta = event.key === 'ArrowDown' ? 1 : -1;
      next = index < 0 ? 0 : (index + delta + items.length) % items.length;
    }
    items[next]?.focus();
  };

  return (
    <div
      ref={rootRef}
      className="tree-context-menu tab-context-menu agent-session-context-menu"
      role="menu"
      aria-label={`Actions for ${session.title || session.id}`}
      style={{ top: position.top, left: position.left }}
      onKeyDown={moveFocus}
      onContextMenu={(event) => event.preventDefault()}
    >
      {confirmingDelete ? (
        <div className="agent-session-menu-confirm">
          <p className="agent-session-menu-confirm-title">
            Delete “{session.title || session.id}”?
          </p>
          <p className="agent-session-menu-confirm-hint">
            This removes the session and its transcript. It cannot be undone.
          </p>
          <div className="agent-session-menu-confirm-actions">
            <button
              type="button"
              role="menuitem"
              className="context-menu-item"
              onClick={() => setConfirmingDelete(false)}
            >
              Cancel
            </button>
            <button
              type="button"
              role="menuitem"
              className="context-menu-item is-danger"
              onClick={() => {
                void Promise.resolve(onDelete()).then(onClose).catch((failure) =>
                  setError(failure instanceof Error ? failure.message : String(failure)));
              }}
            >
              Delete
            </button>
          </div>
        </div>
      ) : (
        <>
          {error && <p role="alert">{error}</p>}
          <button type="button" role="menuitem" className="context-menu-item"
            onClick={() => { void openInAgents(); }}>
            <span>Open in Agents window</span>
          </button>
          {onShowInMain && <button type="button" role="menuitem" className="context-menu-item"
          onClick={() => { void Promise.resolve(onShowInMain()).then(onClose).catch((failure) =>
            setError(failure instanceof Error ? failure.message : String(failure))); }}>Show in main window</button>}
          {onRename && <button type="button" role="menuitem" className="context-menu-item"
            onClick={() => { void Promise.resolve(onRename()).then(onClose).catch((failure) =>
              setError(failure instanceof Error ? failure.message : String(failure))); }}>Rename session</button>}
          {onCopy && <button type="button" role="menuitem" className="context-menu-item"
            onClick={() => { void Promise.resolve(onCopy()).then(onClose).catch((failure) =>
              setError(failure instanceof Error ? failure.message : String(failure))); }}>Copy session ID</button>}
          <button
            type="button"
            role="menuitem"
            className="context-menu-item"
            onClick={() => {
              onTogglePin();
              onClose();
            }}
          >
            <span>{isPinned ? 'Unpin session' : 'Pin session'}</span>
          </button>
          <div className="context-menu-divider" role="separator" />
          <button
            type="button"
            role="menuitem"
            className="context-menu-item is-danger"
            disabled={!canDelete}
            title={canDelete ? undefined : 'Stop the running turn before deleting this session.'}
            onClick={() => setConfirmingDelete(true)}
          >
            <span>Delete session</span>
          </button>
        </>
      )}
    </div>
  );
}
