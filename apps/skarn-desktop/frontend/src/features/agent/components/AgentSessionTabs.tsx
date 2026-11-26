// Selora IDE — Phase 9a Agent session tabs (ROADMAP §"Phase 9", task 9a).
//
// The tabs are a projection of `bootstrap.chat.sessions` (backend truth) and
// nothing else. New/select dispatch the real `NewSession` / `SwitchSession`
// protocol commands through `agentStore`; a rejected command surfaces the
// backend message in the tab strip and never creates a placeholder session.
//
// Keyboard follows the WAI-ARIA tabs pattern: roving tabindex, Left/Right,
// Home/End move focus and activate the backend session.
import React, { useRef, useState } from 'react';
import { Plus } from '../../../components/icons';
import type { AgentSession } from '../types';
import { AgentSessionContextMenu } from './AgentSessionContextMenu';
import './agent-session-tabs-polish.css';

export type AgentSessionTabsProps = {
  sessions: AgentSession[];
  activeSessionId: string;
  busy?: boolean;
  error?: string | null;
  pinnedSessionIds?: string[];
  onNew: () => void;
  onSelect: (sessionId: string) => void;
  onTogglePin?: (sessionId: string) => void;
  onDelete?: (sessionId: string) => void;
};

export const AgentSessionTabs: React.FC<AgentSessionTabsProps> = ({
  sessions,
  activeSessionId,
  busy = false,
  error,
  pinnedSessionIds = [],
  onNew,
  onSelect,
  onTogglePin,
  onDelete,
}) => {
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [menu, setMenu] = useState<{ sessionId: string; x: number; y: number } | null>(null);
  const orderedSessions = [...sessions].sort((a, b) =>
    Number(pinnedSessionIds.includes(b.id)) - Number(pinnedSessionIds.includes(a.id)),
  );
  const menuSession = sessions.find((session) => session.id === menu?.sessionId);

  const focusTab = (index: number) => {
    if (orderedSessions.length === 0) return;
    const bounded = (index + orderedSessions.length) % orderedSessions.length;
    const target = orderedSessions[bounded];
    if (!target) return;
    onSelect(target.id);
    tabRefs.current[bounded]?.focus();
  };

  return (
    <div className="agent-session-tabs-strip agent-sessiontabs-strip">
      <div
        className="agent-session-tabs agent-sessiontabs-list"
        role="tablist"
        aria-label="Agent sessions"
      >
        {sessions.length === 0 ? (
          <p className="agent-session-tabs-empty agent-sessiontabs-empty">No sessions yet</p>
        ) : (
          orderedSessions.map((session, index) => {
            const selected = session.id === activeSessionId;
            const pinned = pinnedSessionIds.includes(session.id);
            return (
              <button
                key={session.id}
                ref={(element) => {
                  tabRefs.current[index] = element;
                }}
                type="button"
                role="tab"
                id={`agent-session-tab-${session.id}`}
                className={`agent-session-tab agent-sessiontabs-tab${selected ? ' is-active' : ''}${pinned ? ' is-pinned' : ''}`}
                aria-selected={selected}
                aria-controls="agent-session-panel"
                tabIndex={selected ? 0 : -1}
                title={session.title}
                onClick={() => onSelect(session.id)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  setMenu({ sessionId: session.id, x: event.clientX, y: event.clientY });
                }}
                onKeyDown={(event) => {
                  if (event.key === 'ArrowRight') {
                    event.preventDefault();
                    focusTab(index + 1);
                  } else if (event.key === 'ArrowLeft') {
                    event.preventDefault();
                    focusTab(index - 1);
                  } else if (event.key === 'Home') {
                    event.preventDefault();
                    focusTab(0);
                  } else if (event.key === 'End') {
                    event.preventDefault();
                    focusTab(orderedSessions.length - 1);
                  } else if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
                    event.preventDefault();
                    const rect = event.currentTarget.getBoundingClientRect();
                    setMenu({ sessionId: session.id, x: rect.left, y: rect.bottom });
                  }
                }}
              >
                <span
                  className={`agent-session-tab-dot agent-sessiontabs-dot${
                    session.isRunning ? ' is-running' : ''
                  }`}
                  aria-hidden="true"
                />
                {pinned && <span className="agent-session-tab-pin" aria-label="Pinned">●</span>}
                <span className="agent-session-tab-title agent-sessiontabs-tab-title">
                  {session.title}
                </span>
              </button>
            );
          })
        )}
      </div>

      <button
        type="button"
        className="agent-session-tab-add agent-sessiontabs-add"
        onClick={onNew}
        disabled={busy}
        aria-busy={busy}
        title="New agent session"
        aria-label="New session"
      >
        <Plus size={12} aria-hidden="true" />
        <span>{busy ? 'Starting…' : 'New'}</span>
      </button>

      {error ? (
        <p className="agent-session-tabs-error agent-sessiontabs-error" role="alert">
          {error}
        </p>
      ) : null}
      {menu && menuSession && onTogglePin && (
        <AgentSessionContextMenu
          key={menuSession.id}
          session={menuSession}
          x={menu.x}
          y={menu.y}
          isPinned={pinnedSessionIds.includes(menuSession.id)}
          canDelete={!menuSession.isRunning && !busy && Boolean(onDelete)}
          onTogglePin={() => onTogglePin(menuSession.id)}
          onDelete={() => onDelete?.(menuSession.id)}
          onClose={() => setMenu(null)}
        />
      )}
    </div>
  );
};
