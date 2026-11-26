// Skarn — Phase Z2 status bar (DESIGN §3.7, ARCHITECTURE §8.1).
//
// Live or Absent policy: every displayed item reflects real live state.
import { useSyncExternalStore } from 'react';
import type { BootstrapState, ConnectionStatus } from '../bridge/bridge';
import { getAppSnapshot, subscribeApp } from '../app/app-store';
import { layoutStore } from '../features/layout/layout-store';
import { agentStore } from '../features/agent/agent-store';
import { editorStore, type EditorBlockingIssue } from '../features/editor/editor-store';
import { SkarnMark } from '../components/SkarnMark';
import {
  GitBranch,
  DangerIcon,
  AlertCircle,
} from '../components/icons';

const subscribeEditorStore = (listener: () => void) =>
  typeof editorStore.subscribe === 'function' ? editorStore.subscribe(listener) : () => undefined;

const EMPTY_BLOCKING_ISSUES: EditorBlockingIssue[] = [];
const getBlockingIssuesSnapshot = () =>
  typeof editorStore.getSnapshot === 'function'
    ? editorStore.getSnapshot().blockingIssues
    : EMPTY_BLOCKING_ISSUES;

function ConnectionStatusView({
  status,
  connectionError,
  onOpenConnect,
}: {
  status: ConnectionStatus;
  connectionError: string | null;
  onOpenConnect?: () => void;
}) {
  if (status === 'connected') {
    return (
      <button
        type="button"
        className="statusbar-connection is-connected"
        onClick={onOpenConnect}
        disabled={!onOpenConnect}
        title={onOpenConnect ? 'Provider connected (click to manage API key / provider)' : undefined}
      >
        <span className="statusbar-radar-wrapper" aria-hidden="true">
          <span className="statusbar-radar-ring" />
          <span className="statusbar-dot" />
        </span>
        Provider connected
      </button>
    );
  }

  if (connectionError) {
    const isKeyringError = /keyring|credential manager/i.test(connectionError);
    return (
      <span className="statusbar-connection is-disconnected is-error">
        <span className="statusbar-dot" aria-hidden="true" />
        <button
          type="button"
          className="statusbar-connect"
          disabled={!onOpenConnect}
          onClick={onOpenConnect}
          title={connectionError}
        >
          {isKeyringError ? 'Keyring error · Re-enter key' : 'Connection error · Retry'}
        </button>
      </span>
    );
  }

  const label = status === 'connecting' ? 'Provider connecting' : 'Provider disconnected';
  return (
    <span className="statusbar-connection is-disconnected">
      <span className="statusbar-dot" aria-hidden="true" />
      {label}
      <span className="statusbar-separator" aria-hidden="true">
        ·
      </span>
      <button
        type="button"
        className="statusbar-connect"
        disabled={!onOpenConnect}
        onClick={onOpenConnect}
        title={onOpenConnect ? 'Connect API key / provider' : undefined}
      >
        Connect
      </button>
    </span>
  );
}

export function StatusBar({
  bootstrap,
  onOpenConnect,
}: {
  bootstrap: BootstrapState;
  onOpenConnect?: () => void;
}) {
  const appState = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const agentState = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const blockingIssues = useSyncExternalStore(subscribeEditorStore, getBlockingIssuesSnapshot);

  const gitBranch = bootstrap.git?.branch || null;

  // Active buffer language (absent when no buffer is open)
  const activeBuffer = bootstrap.buffers?.find((b) => b.buffer_id === bootstrap.active_buffer_id);
  const activeLanguage = activeBuffer?.language
    ? activeBuffer.language.charAt(0).toUpperCase() + activeBuffer.language.slice(1)
    : null;

  // Diagnostics summary
  const errorCount = bootstrap.diagnostics?.filter((d) => d.severity === 'error').length ?? 0;
  const warningCount = bootstrap.diagnostics?.filter((d) => d.severity === 'warning').length ?? 0;

  // Agent activity
  const agentInFlight = agentState.inFlight || agentState.thinkingState !== 'breathing';
  const agentWaiting = Boolean(bootstrap.chat?.pending_approval);
  const agentLabel = agentInFlight
    ? `Agent ${agentState.thinkingState.charAt(0).toUpperCase() + agentState.thinkingState.slice(1)}...`
    : agentWaiting
    ? 'Approval Required'
    : 'Agent';

  const firstBlockingIssue = blockingIssues[0] ?? null;

  return (
    <footer className="statusbar">
      {/* Left zone: Workspace, live Git branch, active language */}
      <div className="statusbar-zone statusbar-left">
        <button
          type="button"
          className="statusbar-item statusbar-workspace"
          onClick={() => void layoutStore.toggle('files')}
          title={`Workspace: ${bootstrap.workspace.display_name} (click to toggle Files)`}
        >
          <SkarnMark size={11} />
          <span>{bootstrap.workspace.display_name}</span>
        </button>

        {gitBranch ? (
          <button
            type="button"
            className="statusbar-item statusbar-git"
            aria-label={`Git branch ${gitBranch}`}
            onClick={() => void layoutStore.toggle('git')}
            title={`Git branch: ${gitBranch} (click to toggle Git)`}
          >
            <GitBranch size={11} className="git-icon" aria-hidden="true" />
            <span>{gitBranch}</span>
          </button>
        ) : null}

        {activeLanguage ? (
          <span className="statusbar-item statusbar-lang" title="Active Language Mode">
            <span>{activeLanguage}</span>
          </span>
        ) : null}
      </div>

      {/* Center zone: Agent activity pill / status */}
      <div className="statusbar-zone statusbar-center">
        <button
          type="button"
          className={`statusbar-agent-pill ${agentInFlight ? 'is-active' : ''}`}
          onClick={() => void layoutStore.toggle('agent')}
          title="Agent Workbench Status (click to toggle Agent Dock Ctrl+L)"
          aria-label="Agent Status"
        >
          <span
            className={`agent-radar-dot ${
              agentInFlight ? 'is-thinking' : agentWaiting ? 'is-waiting' : ''
            }`}
            aria-hidden="true"
          />
          <span className="agent-status-label">{agentLabel}</span>
        </button>
        <span className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
          {agentLabel}
        </span>
      </div>

      {/* Right zone: Blocking file errors, LSP diagnostics, Connection status */}
      <div className="statusbar-zone statusbar-right">
        {firstBlockingIssue ? (
          <button
            type="button"
            className="statusbar-item statusbar-blocking"
            role="status"
            title={firstBlockingIssue.detail}
            onClick={() => editorStore.setActive?.(firstBlockingIssue.bufferId)}
          >
            <DangerIcon size={11} aria-hidden="true" />
            <span>
              {blockingIssues.length === 1
                ? `${firstBlockingIssue.filename}: ${firstBlockingIssue.summary}`
                : `${blockingIssues.length} files need sync`}
            </span>
          </button>
        ) : null}

        <button
          type="button"
          className="statusbar-item statusbar-diagnostics"
          aria-label="Problems summary"
          onClick={() => void layoutStore.toggle('problems')}
          title={`Diagnostics: ${errorCount} errors, ${warningCount} warnings (click to toggle Problems)`}
        >
          <span className="diag-error">
            <DangerIcon size={11} aria-hidden="true" />
            <span>{errorCount}</span>
          </span>
          <span className="diag-warning">
            <AlertCircle size={11} aria-hidden="true" />
            <span>{warningCount}</span>
          </span>
        </button>

        <ConnectionStatusView
          status={bootstrap.connection}
          connectionError={appState?.connectionError ?? null}
          onOpenConnect={onOpenConnect}
        />
      </div>
    </footer>
  );
}
