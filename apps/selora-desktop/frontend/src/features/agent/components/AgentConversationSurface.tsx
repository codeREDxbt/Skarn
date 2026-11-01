// Selora IDE — Phase 9a Agent Workbench (ROADMAP §"Phase 9", task 9a).
// The dedicated agent surface: backend session tabs, transcript reuse
// (AgentExecutionFeed's cards/thinking states), a live todo spine, structured
// question / plan review cards inside ToolCard, the composer's Ask / Agent /
// Plan row, and an explicit way back to the editor or split view.
//
// Every session and todo shown here comes from Rust (`bootstrap.chat`) or a
// real protocol event. New/select go through the real NewSession /
// SwitchSession commands; a rejected command shows its message and creates
// nothing locally.
import React, { useEffect, useState, useSyncExternalStore } from 'react';
import { ChevronDown, Code2, Columns, ShieldCheck, Square } from '../../../components/icons';
import { agentStore, getComposerGate } from '../agent-store';
import { AgentSessionTabs } from './AgentSessionTabs';
import { AgentSessionContextMenu } from './AgentSessionContextMenu';
import { AgentSessionRail } from './AgentSessionRail';
import { AgentExecutionFeed } from './AgentExecutionFeed';
import { AgentPlanSpine } from './AgentPlanSpine';
import { AgentComposer } from './AgentComposer';
import { SubagentResults } from './SubagentResults';
import { SubagentSessionView } from './SubagentSessionView';
import { ThinkingOrb } from '../presence/ThinkingOrb';
import { BrandMark } from '../../onboarding/BrandFx';
import '../../../styles/onboarding.css';
import './agent-welcome.css';
import { activeEditorContext, editorStore } from '../../editor/editor-store';
import { openAgentsWindow, type BootstrapState } from '../../../bridge/bridge';

const SUGGESTIONS = [
  'Explain this file',
  'Find a bug in the selection',
  'Summarize the workspace',
];

export type AgentConversationSurfaceProps = {
  bootstrap?: BootstrapState;
  presentation?: 'sidebar' | 'agents';
  /** Leave the dedicated agent view for the editor or split shell. */
  onExit?: (mode: 'editor' | 'split') => void;
};

/** Shared composition; sidebar chrome remains exclusive to the main window. */
export function AgentConversationSurface({ bootstrap, onExit, presentation = 'sidebar' }: AgentConversationSurfaceProps = {}) {
  const state = useSyncExternalStore(
    agentStore.subscribe,
    agentStore.getSnapshot
  );
  const editor = useSyncExternalStore(
    editorStore.subscribe,
    editorStore.getSnapshot
  );

  const [activeTab, setActiveTab] = useState<'feed' | 'plan'>('feed');
  const [showFloatingPlan, setShowFloatingPlan] = useState(false);
  const [infeedPlanExpanded, setInfeedPlanExpanded] = useState(false);
  const [showSessionRail, setShowSessionRail] = useState(false);
  const [windowError, setWindowError] = useState<string | null>(null);
  const [sessionMenu, setSessionMenu] = useState<{ sessionId: string; x: number; y: number } | null>(null);

  const activeSession =
    state.sessions.find((session) => session.id === state.activeSessionId) ?? null;
  const menuSession = state.sessions.find((session) => session.id === sessionMenu?.sessionId);

  const hasMessages = state.messages.length > 0;
  const completedTodoCount = state.todos.filter((todo) => todo.status === 'completed').length;
  const currentTask = state.todos.find((todo) => todo.status === 'in_progress')?.text;
  // Only a backend-reported workspace with a real root earns a name line.
  const workspaceName =
    state.workspace?.root &&
    state.workspace.displayName &&
    state.workspace.displayName !== 'No workspace open'
      ? state.workspace.displayName
      : null;
  const gate = getComposerGate(state);
  const autoApplyArmed = state.permissionMode === 'full';
  const activeSessionTabId = activeSession
    ? `agent-session-tab-${activeSession.id}`
    : undefined;

  const exitWorkbench = (mode: 'editor' | 'split') => {
    if (onExit) {
      onExit(mode);
      return;
    }
    // The shell listens for this when AgentDock is rendered outside ShellGrid
    // (tests, isolated previews). One event, no local layout fiction.
    if (typeof window !== 'undefined') {
      window.dispatchEvent(
        new CustomEvent('selora:set-view-mode', { detail: { mode } }),
      );
    }
  };
  const activeChild = state.activeSubagentId
    ? state.subagents.find((subagent) =>
        subagent.id === state.activeSubagentId && subagent.parentSessionId === state.activeSessionId,
      )
    : undefined;

  useEffect(() => {
    if (state.revealedSubagentId) setActiveTab('feed');
  }, [state.revealedSubagentId]);

  useEffect(() => {
    setInfeedPlanExpanded(false);
  }, [state.activeSessionId]);

  const suggestions = SUGGESTIONS.filter((suggestion) => {
    if (presentation === 'agents') return suggestion === 'Summarize the workspace';
    if (suggestion === 'Explain this file') return editor.activeBuffer !== null;
    if (suggestion === 'Find a bug in the selection') return editor.activeSelection !== null;
    return true;
  });

  return (
    <div className={`agent-dock agent-workbench-root${presentation === 'agents' ? ' agents-conversation-surface' : ''}`} data-presentation={presentation} data-conversation-state={hasMessages ? 'active' : 'welcome'} role="region" aria-label="Agent workbench">
      {/* 1. Workbench header: presence, active session badge, view actions. */}
      {presentation === 'sidebar' && <>
      <header className="agent-header agent-workbench-header">
        <div className="header-left-group">
          <span className="agent-presence-slot" aria-hidden="true">
            <ThinkingOrb state={state.thinkingState} size={20} />
          </span>

          <span className="agent-session-label">Agent</span>

          <span
            className="agent-session-badge"
            role="status"
            onContextMenu={(event) => {
              if (!activeSession) return;
              event.preventDefault();
              setSessionMenu({ sessionId: activeSession.id, x: event.clientX, y: event.clientY });
            }}
            title={
              activeSession
                ? `Active session: ${activeSession.title}`
                : 'The backend has not reported an agent session yet'
            }
          >
            <span
              className={`agent-session-badge-dot${state.inFlight ? ' is-live' : ''}`}
              aria-hidden="true"
            />
            <span className="agent-session-badge-title">
              {activeSession ? activeSession.title : 'No session'}
            </span>
          </span>
        </div>

        <div className="header-right-group">
          <button type="button" className="agent-exit-btn" aria-label="Open Agents window"
            title="Open this conversation in the Agents window"
            onClick={() => {
              setWindowError(null);
               void Promise.resolve(openAgentsWindow({ sessionId: state.activeSessionId ?? undefined,
                 childId: state.activeSubagentId ?? undefined, workspaceId: state.workspace?.id }))
                 .catch((error: unknown) => setWindowError(error instanceof Error ? error.message : String(error)));
            }}>
            <Columns size={12} aria-hidden="true" />
          </button>
          {autoApplyArmed && (
            <span
              className="agent-auto-apply-badge"
              role="status"
              title="Full auto-apply is active: file writes, edits and deletes apply immediately for this session. Bash commands still require approval."
            >
              <ShieldCheck size={11} aria-hidden="true" />
              Auto-apply armed
            </span>
          )}

          <button
            type="button"
            className={`agent-view-tab ${activeTab === 'feed' ? 'is-active' : ''}`}
            onClick={() => setActiveTab('feed')}
            aria-pressed={activeTab === 'feed'}
            disabled={Boolean(activeChild)}
            title={activeChild ? 'Back to parent to leave the child session' : undefined}
          >
            Chat
          </button>
          <button
            type="button"
            className={`agent-view-tab ${activeTab === 'plan' ? 'is-active' : ''}`}
            onClick={() => setActiveTab('plan')}
            aria-pressed={activeTab === 'plan'}
            disabled={Boolean(activeChild)}
            title={activeChild ? 'Back to parent to leave the child session' : undefined}
          >
            Plan ({state.todos.filter((t) => t.status === 'completed').length}/
            {state.todos.length})
          </button>

          {activeTab === 'feed' && !activeChild && (
            <button
              type="button"
              className={`agent-floating-plan-toggle ${showFloatingPlan ? 'is-active' : ''}`}
              onClick={() => setShowFloatingPlan(!showFloatingPlan)}
              title={showFloatingPlan ? 'Hide floating plan card' : 'Show floating plan card'}
              aria-label="Toggle floating plan"
              aria-pressed={showFloatingPlan}
            >
              <svg
                width="13"
                height="13"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
                aria-hidden="true"
                focusable="false"
              >
                <rect x="2" y="2" width="12" height="12" rx="2" />
                <path d="M5 8h6M5 5h6M5 11h4" />
              </svg>
            </button>
          )}

          <button
            type="button"
            className="agent-header-stop"
            onClick={() => agentStore.stopActiveTurn()}
            disabled={!state.inFlight}
            title={state.inFlight ? 'Stop the running agent turn (Esc)' : 'No agent turn is running'}
            aria-label="Stop agent run"
          >
            <Square size={11} aria-hidden="true" />
            <span>Stop</span>
          </button>

          <div className="agent-exit-group" role="group" aria-label="Switch workspace view">
            <button
              type="button"
              className="agent-exit-btn"
              onClick={() => exitWorkbench('editor')}
              title="Return to the editor view"
              aria-label="Editor view"
            >
              <Code2 size={12} aria-hidden="true" />
              <span>Editor</span>
            </button>
            <button
              type="button"
              className="agent-exit-btn"
              onClick={() => exitWorkbench('split')}
              title="Show the agent beside the editor"
              aria-label="Split view"
            >
              <Columns size={12} aria-hidden="true" />
              <span>Split</span>
            </button>
          </div>
        </div>
      </header>
      {windowError && <p className="agent-session-error" role="alert">{windowError}</p>}

      {/* 2. Backend session tabs (new / select via real protocol commands). */}
      <AgentSessionTabs
        sessions={state.sessions}
        activeSessionId={state.activeSessionId}
        busy={state.sessionBusy}
        error={state.sessionError}
        pinnedSessionIds={state.pinnedSessionIds}
        onNew={() => void agentStore.requestNewSession()}
        onSelect={(sessionId) => void agentStore.requestSwitchSession(sessionId)}
        onTogglePin={(sessionId) => agentStore.toggleSessionPinned(sessionId)}
        onDelete={(sessionId) => void agentStore.requestDeleteSession(sessionId)}
      />
      </>}
      {/* Phase 9e: running and finished children under the selected parent.
          Opening one swaps the body below for its read-only transcript. */}
      <SubagentResults
        sessionId={state.activeSessionId}
        subagents={state.subagents}
        revealedChildId={state.revealedSubagentId}
        onRead={agentStore.markSubagentRead}
        onClearReveal={agentStore.clearSubagentReveal}
        onOpenSession={(childId) => {
          if (agentStore.openSubagentSession(childId)) setActiveTab('feed');
        }}
      />

      {/* Durable sessions share the same backend switch route as the tabs. */}
      {presentation === 'sidebar' && <>
        <button type="button" className="agent-decision-secondary" aria-expanded={showSessionRail}
          onClick={() => setShowSessionRail(!showSessionRail)}>Session history</button>
      {showSessionRail && <AgentSessionRail sessions={state.sessions} activeSessionId={state.activeSessionId}
        resumableSessions={state.resumableSessions} onClose={() => setShowSessionRail(false)}
        onDelete={(sessionId) => void agentStore.requestDeleteSession(sessionId)} />}
      </>}

      {/* 3. Workbench body */}
      <div
        className={`agent-workbench-body${activeChild ? ' has-subagent-session' : ''}`}
        id={presentation === 'sidebar' ? 'agent-session-panel' : 'agents-session-panel'}
        role={presentation === 'sidebar' ? 'tabpanel' : undefined}
        aria-labelledby={presentation === 'sidebar' ? activeSessionTabId : undefined}
      >
        <div className="agent-parent-layer" aria-hidden={activeChild ? true : undefined}>
        {activeTab === 'feed' ? (
          <div className="agent-feed-column">
            {showFloatingPlan && (
              <div className="zcode-floating-plan-popover">
                <AgentPlanSpine
                  todos={state.todos}
                  gitStatus={bootstrap?.git}
                  isFloatingPopover={true}
                  onClosePopover={() => setShowFloatingPlan(false)}
                />
              </div>
            )}

            <div className="agent-chat-content">
              {/* Compact collapsible todo progress bar */}
              {state.todos.length > 0 && (
                <div className={`zcode-infeed-progress-wrap${infeedPlanExpanded ? ' is-expanded' : ''}`}>
                  <button
                    type="button"
                    className="zcode-infeed-progress-toggle"
                    onClick={() => setInfeedPlanExpanded(!infeedPlanExpanded)}
                    aria-expanded={infeedPlanExpanded}
                    aria-controls="zcode-infeed-progress-body"
                    title={currentTask ? `Current: ${currentTask}` : undefined}
                  >
                    <span className="infeed-left">
                      <span className="infeed-label">
                        Tasks ({completedTodoCount}/{state.todos.length})
                      </span>
                      {currentTask ? (
                        <>
                          <span className="infeed-bullet" aria-hidden="true">•</span>
                          <span className="infeed-current-task">{currentTask}</span>
                        </>
                      ) : null}
                    </span>
                    <span className="infeed-right">
                      <span className="infeed-action">
                        {infeedPlanExpanded ? 'Collapse' : 'View tasks'}
                      </span>
                      <ChevronDown
                        size={12}
                        aria-hidden="true"
                        style={{
                          transform: infeedPlanExpanded ? 'rotate(180deg)' : 'none',
                          transition: 'transform 150ms ease',
                        }}
                      />
                    </span>
                  </button>
                  {infeedPlanExpanded && (
                    <div id="zcode-infeed-progress-body" className="zcode-infeed-expanded-body">
                      <AgentPlanSpine
                        todos={state.todos}
                        gitStatus={bootstrap?.git}
                        isCompactInfeed={true}
                      />
                    </div>
                  )}
                </div>
              )}

              {hasMessages ? (
                <AgentExecutionFeed
                  scrollKey={`${state.workspace?.id ?? ''}:${state.activeSessionId ?? ''}:${presentation}`}
                  surface={presentation === 'agents' ? 'agents' : 'main'}
                  messages={state.messages}
                  inFlight={state.inFlight}
                  thinkingState={state.thinkingState}
                  metrics={state.metrics}
                  promptQueue={state.promptQueue}
                />
              ) : (
                <div className="agent-body">
                  <div className="agent-welcome">
                    <div className="agent-welcome-brand" aria-hidden="true">
                      <div className="agent-welcome-brand-ambient" />
                      <div className="agent-welcome-brand-stage">
                        <BrandMark className="agent-welcome-brand-mark" />
                        <span className="agent-welcome-pulse-ring" />
                      </div>
                    </div>
                    <h2 className="agent-welcome-heading">What are we building today?</h2>
                    <p className="agent-welcome-sub">Ask about the code, or describe a change.</p>
                    {workspaceName ? (
                      <p className="agent-welcome-workspace" title={workspaceName}>
                        {workspaceName}
                      </p>
                    ) : null}
                    <div className="agent-suggestions" role="group" aria-label="Suggested prompts">
                      {suggestions.map((suggestion) => (
                        <button
                          key={suggestion}
                          type="button"
                          className="agent-chip"
                          disabled={!gate.ready}
                          title={gate.ready ? undefined : gate.message ?? undefined}
                          onClick={() => {
                             const context = presentation === 'agents' ? undefined : activeEditorContext();
                            if (context) agentStore.sendUserMessage(suggestion, false, context);
                            else agentStore.sendUserMessage(suggestion);
                          }}
                        >
                          {suggestion}
                        </button>
                      ))}
                    </div>
                  </div>
                </div>
              )}
            </div>

            <AgentComposer
              surface={presentation === 'agents' ? 'agents' : 'main'}
              onPreviewAttachment={presentation === 'agents' ? (path) => window.dispatchEvent(new CustomEvent('selora:preview-agent-file', {
                detail: { path, workspaceId: state.workspace?.id, sessionId: state.activeSessionId, surface: 'agents' },
              })) : undefined}
              inFlight={state.inFlight}
              streamingActivityLevel={state.streamingActivityLevel}
              permissionMode={state.permissionMode}
              reasoningEffort={state.reasoningEffort}
              promptHistory={state.promptHistory}
            />
          </div>
        ) : (
<div className="agent-feed-column">
            <div className="agent-plan-scroll">
              <AgentPlanSpine
                todos={state.todos}
                gitStatus={bootstrap?.git}
              />
            </div>

            <AgentComposer
              surface={presentation === 'agents' ? 'agents' : 'main'}
              inFlight={state.inFlight}
              streamingActivityLevel={state.streamingActivityLevel}
              permissionMode={state.permissionMode}
              reasoningEffort={state.reasoningEffort}
              promptHistory={state.promptHistory}
            />
          </div>
        )}
        </div>
        {activeChild && (
          <div className="subagent-session-layer">
            <SubagentSessionView
              child={activeChild}
              parentTitle={activeSession?.title}
              onBack={() => agentStore.closeSubagentSession()}
            />
          </div>
        )}
      </div>
      {sessionMenu && menuSession && (
        <AgentSessionContextMenu
          key={menuSession.id}
          session={menuSession}
          x={sessionMenu.x}
          y={sessionMenu.y}
          isPinned={state.pinnedSessionIds.includes(menuSession.id)}
          canDelete={!menuSession.isRunning && !state.sessionBusy}
          onTogglePin={() => agentStore.toggleSessionPinned(menuSession.id)}
          onDelete={() => void agentStore.requestDeleteSession(menuSession.id)}
          onClose={() => setSessionMenu(null)}
        />
      )}
    </div>
  );
}
