import { useEffect, useState, useSyncExternalStore } from 'react';
import { currentEnvelopeContext } from '../../app/app-store';
import { openAgentsWindow, renameAgentSession, showSessionInMainWindow } from '../../bridge/bridge';
import { agentStore } from '../agent/agent-store';
import { AgentSessionContextMenu } from '../agent/components/AgentSessionContextMenu';
import {
  Add,
  CloseIcon,
  DirectSend,
  GitBranch,
  Messages1,
  More,
  Search,
  Setting2,
  Zap,
} from '../../components/icons';
import {
  filterNavigationSessions, formatSessionDate, groupNavigationSessions, mergeNavigationSessions, sessionAttention,
  type NavigationSession, type SessionFilter,
} from './session-navigation';
import './agents-navigation-rail.css';

export type AgentsNavigationRailProps = {
  page: 'conversation' | 'automations' | 'customizations';
  onNavigate: (page: AgentsNavigationRailProps['page']) => void;
  onSelectSession?: (id: string) => void;
};

export function AgentsNavigationRail({ page, onNavigate, onSelectSession }: AgentsNavigationRailProps) {
  const state = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<SessionFilter>('all');
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [rename, setRename] = useState<{ id: string; title: string } | null>(null);
  const attention = sessionAttention(state.transcripts);
  const rows = mergeNavigationSessions(state.sessions, state.resumableSessions, state.pinnedSessionIds, [...attention.keys()]);
  const groups = groupNavigationSessions(filterNavigationSessions(rows, query, filter));
  const target = rows.find((row) => row.id === menu?.id);
  const busy = pending || state.sessionBusy;

  useEffect(() => {
    let mounted = true;
    setHistoryLoading(true);
    setHistoryError(null);
    void agentStore.refreshAgents().catch((failure: unknown) => {
      if (mounted) setHistoryError(failure instanceof Error ? failure.message : String(failure));
    }).finally(() => { if (mounted) setHistoryLoading(false); });
    return () => { mounted = false; };
  }, [state.workspace?.id]);

  const perform = async (action: () => Promise<unknown>) => {
    setError(null);
    setPending(true);
    try { await action(); }
    catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
    finally { setPending(false); }
  };
  const contextFor = (id: string) => {
    const context = currentEnvelopeContext();
    if (!context) throw new Error('Open a workspace before managing agent sessions.');
    return { ...context, session_id: id };
  };
  const select = (id: string) => perform(async () => {
    if (await agentStore.requestSwitchSession(id)) {
      onSelectSession?.(id);
      onNavigate('conversation');
    }
  });
  const openWindow = (id: string) => perform(async () => {
    const context = contextFor(id);
    await openAgentsWindow({ sessionId: id, workspaceId: context.workspace_id });
  });
  const openMenu = (row: NavigationSession, x: number, y: number) => {
    setMenu({ id: row.id, x, y });
  };

  const runningCount = rows.filter((r) => r.isRunning).length;
  const attentionCount = rows.filter((r) => r.needsAttention).length;
  const pinnedCount = rows.filter((r) => r.pinned).length;

  return (
    <aside className="agents-navigation-rail" aria-label="Agents navigation">
      <nav aria-label="Agents pages">
        <button className="agents-nav-new-btn" type="button" disabled={busy} onClick={() => void perform(async () => {
          if (await agentStore.requestNewSession()) onNavigate('conversation');
        })}>
          <Add size={14} aria-hidden="true" />
          <span>New Session</span>
        </button>
        {(['conversation', 'automations', 'customizations'] as const).map((item) => (
          <button key={item} className="agents-nav-page-btn" type="button" aria-current={page === item ? 'page' : undefined}
            onClick={() => onNavigate(item)}>
            {item === 'conversation' ? <><DirectSend size={14} aria-hidden="true" /><span>Conversations</span></>
              : item === 'automations' ? <><Zap size={14} aria-hidden="true" /><span>Automations</span></>
              : <><Setting2 size={14} aria-hidden="true" /><span>Customizations</span></>}
          </button>
        ))}
      </nav>
      <h2 className="agents-navigation-heading">Sessions</h2>
      <label className="agents-navigation-search">
        <span className="sr-only">Search sessions</span>
        <div className="agents-nav-search-wrap">
          <Search size={14} className="agents-nav-search-icon" aria-hidden="true" />
          <input type="search" value={query} placeholder="Search sessions…" onChange={(event) => setQuery(event.target.value)} />
          {query.length > 0 && (
            <button type="button" className="agents-nav-search-clear" aria-label="Clear search" onClick={() => setQuery('')}>
              <CloseIcon size={12} aria-hidden="true" />
            </button>
          )}
        </div>
      </label>
      <div className="agents-navigation-filter-bar">
        <label className="agents-navigation-filter sr-only">
          <span>Filter sessions</span>
          <select value={filter} onChange={(event) => setFilter(event.target.value as SessionFilter)}>
            <option value="all">All sessions</option>
            <option value="running">Running</option>
            <option value="attention">Needs attention</option>
            <option value="pinned">Pinned</option>
          </select>
        </label>
        <div className="agents-nav-filter-chips" role="group" aria-label="Quick filters">
          <button type="button" className={`agents-nav-chip${filter === 'all' ? ' is-active' : ''}`} onClick={() => setFilter('all')}>
            <span>All</span>
            <span className="agents-nav-chip-count">{rows.length}</span>
          </button>
          {runningCount > 0 && (
            <button type="button" className={`agents-nav-chip${filter === 'running' ? ' is-active' : ''}`} onClick={() => setFilter('running')}>
              <span className="agents-nav-dot-pulse" aria-hidden="true" />
              <span>Running</span>
              <span className="agents-nav-chip-count">{runningCount}</span>
            </button>
          )}
          {attentionCount > 0 && (
            <button type="button" className={`agents-nav-chip${filter === 'attention' ? ' is-active' : ''}`} onClick={() => setFilter('attention')}>
              <span>Attention</span>
              <span className="agents-nav-chip-count">{attentionCount}</span>
            </button>
          )}
          {pinnedCount > 0 && (
            <button type="button" className={`agents-nav-chip${filter === 'pinned' ? ' is-active' : ''}`} onClick={() => setFilter('pinned')}>
              <span>Pinned</span>
              <span className="agents-nav-chip-count">{pinnedCount}</span>
            </button>
          )}
        </div>
      </div>
      {(error || state.sessionError) && <p role="alert">{error || state.sessionError}</p>}
      {historyError && (
        <div className="agents-nav-error-banner" role="alert">
          <span>Could not load session history: {historyError}</span>
          <button type="button" className="agents-nav-error-retry" onClick={() => void agentStore.refreshAgents()}>Retry</button>
        </div>
      )}
      <div className="agents-navigation-sessions" aria-busy={busy || historyLoading}>
        {historyLoading && (
          <>
            <p role="status">Loading session history…</p>
            <div className="agents-nav-skeleton-wrap" aria-hidden="true">
              <div className="agents-nav-skeleton-card" />
              <div className="agents-nav-skeleton-card" />
              <div className="agents-nav-skeleton-card" />
            </div>
          </>
        )}
        {!historyLoading && !historyError && groups.length === 0 &&
          <p>{rows.length ? 'No matching sessions' : 'No sessions yet'}</p>}
        {groups.map((group) => (
          <section key={group.label} aria-label={group.label}>
            <h3>
              <span>{group.label}</span>
              <span className="agents-nav-group-count">{group.sessions.length}</span>
            </h3>
            {group.sessions.map((row) => (
              <div className={`agents-navigation-session${page === 'conversation' && state.activeSessionId === row.id ? ' is-active' : ''}`} key={row.id}>
                <button type="button" disabled={busy}
                  aria-current={page === 'conversation' && state.activeSessionId === row.id ? 'page' : undefined}
                  aria-busy={row.isRunning || undefined}
                  title={row.pending?.summary || row.title}
                  onClick={() => void select(row.id)}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    openMenu(row, event.clientX, event.clientY);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
                      event.preventDefault();
                      const rect = event.currentTarget.getBoundingClientRect();
                      openMenu(row, rect.left, rect.bottom);
                    }
                  }}>
                  <div className="agents-nav-session-main">
                    <span className="agents-nav-session-title">{row.title}</span>
                    {row.isRunning && (
                      <small className="agents-nav-badge agents-nav-badge--running">
                        <span className="agents-nav-dot-pulse" aria-hidden="true" />
                        Running
                      </small>
                    )}
                    {attention.get(row.id) === 'error' ? (
                      <small className="agents-nav-badge agents-nav-badge--error">Failed</small>
                    ) : attention.get(row.id) === 'approval' || row.pending?.kind === 'approval' ? (
                      <small className="agents-nav-badge agents-nav-badge--approval">Needs approval</small>
                    ) : row.pending ? (
                      <small className="agents-nav-badge agents-nav-badge--approval">Needs attention · {row.pending.kind}</small>
                    ) : null}
                  </div>
                  <div className="agents-nav-session-meta">
                    {row.updatedAt !== undefined && (() => {
                      const date = formatSessionDate(row.updatedAt);
                      return <time className="agents-navigation-date" dateTime={date.dateTime}
                        title={date.tooltip}>{date.label}</time>;
                    })()}
                    {row.branchName && (
                      <span className="agents-nav-meta-item agents-nav-meta-branch" title={`Branch: ${row.branchName}`}>
                        <GitBranch size={10} aria-hidden="true" />
                        <span>{row.branchName}</span>
                      </span>
                    )}
                    {row.messageCount !== undefined && row.messageCount > 0 && (
                      <span className="agents-nav-meta-item agents-nav-meta-count">
                        <Messages1 size={10} aria-hidden="true" />
                        <span>{row.messageCount} msg{row.messageCount === 1 ? '' : 's'}</span>
                      </span>
                    )}
                  </div>
                </button>
                <button type="button" className="agents-nav-action-btn" disabled={busy} aria-label={`Actions for ${row.title}`}
                  onClick={(event) => {
                    const rect = event.currentTarget.getBoundingClientRect();
                    openMenu(row, rect.left, rect.bottom);
                  }}>
                  <More size={14} aria-hidden="true" />
                </button>
              </div>
            ))}
          </section>
        ))}
      </div>
      {rename && (
        <form className="agents-navigation-rename" aria-label="Rename session" onSubmit={(event) => {
          event.preventDefault();
          const title = rename.title.trim();
          if (!title || busy) return;
          void perform(async () => {
            await renameAgentSession(rename.id, title, contextFor(rename.id));
            agentStore.refreshResumableSessions();
            setRename(null);
          });
        }}>
          <label>Session title<input autoFocus value={rename.title}
            onChange={(event) => setRename({ ...rename, title: event.target.value })} /></label>
          <button type="submit" disabled={busy || !rename.title.trim()}>Save</button>
          <button type="button" disabled={busy} onClick={() => setRename(null)}>Cancel</button>
        </form>
      )}
      {menu && target && <AgentSessionContextMenu key={target.id} session={target}
        x={menu.x} y={menu.y} isPinned={target.pinned} canDelete={!busy && !target.isRunning}
        onTogglePin={() => agentStore.toggleSessionPinned(target.id)}
        onDelete={() => { void perform(async () => {
          if (await agentStore.requestDeleteSession(target.id)) agentStore.refreshResumableSessions();
        }); }}
        onRename={() => setRename({ id: target.id, title: target.title })}
        onCopy={() => { void perform(async () => { await navigator.clipboard.writeText(target.id); }); }}
        onShowInMain={() => { void perform(async () => {
          await showSessionInMainWindow(target.id, contextFor(target.id));
        }); }}
        onOpenInAgents={(id) => openWindow(id)} onClose={() => setMenu(null)} />}
    </aside>
  );
}
