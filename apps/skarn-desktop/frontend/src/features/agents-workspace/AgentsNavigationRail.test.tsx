import React from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AgentsNavigationRail } from './AgentsNavigationRail';
import { AgentSessionContextMenu } from '../agent/components/AgentSessionContextMenu';
import type { ChatMessage } from '../agent/types';

const mocks = vi.hoisted(() => ({
  state: {
    sessions: [{ id: 'live', title: 'Live session', isRunning: true }],
    resumableSessions: [
      { id: 'live', title: 'Durable duplicate', updatedAt: 1000 },
      { id: 'saved', title: 'Saved session', updatedAt: 1000 },
    ],
    pinnedSessionIds: ['saved'], activeSessionId: 'live', sessionBusy: false, sessionError: null,
    transcripts: {} as Record<string, ChatMessage[]>,
  },
  switchSession: vi.fn(), newSession: vi.fn(), deleteSession: vi.fn(), pin: vi.fn(), refresh: vi.fn(),
  rename: vi.fn(), open: vi.fn(), show: vi.fn(), copy: vi.fn(), refreshAgents: vi.fn(),
}));

vi.mock('../agent/agent-store', () => ({ agentStore: {
  subscribe: () => () => {}, getSnapshot: () => mocks.state,
  requestSwitchSession: mocks.switchSession, requestNewSession: mocks.newSession,
  requestDeleteSession: mocks.deleteSession, toggleSessionPinned: mocks.pin,
  refreshResumableSessions: mocks.refresh,
  refreshAgents: mocks.refreshAgents,
} }));
vi.mock('../../app/app-store', () => ({ currentEnvelopeContext: () => ({
  workspace_id: 'workspace', core_instance_id: 'core', session_id: 'wrong',
}) }));
vi.mock('../../bridge/bridge', () => ({
  renameAgentSession: mocks.rename, openAgentsWindow: mocks.open, showSessionInMainWindow: mocks.show,
}));

beforeEach(() => {
  vi.resetAllMocks();
  mocks.switchSession.mockResolvedValue(true);
  mocks.newSession.mockResolvedValue(true);
  mocks.refreshAgents.mockResolvedValue(undefined);
  mocks.state.transcripts = {};
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: mocks.copy } });
});
afterEach(cleanup);

function setup() {
  const navigate = vi.fn();
  const select = vi.fn();
  render(<AgentsNavigationRail page="conversation" onNavigate={navigate} onSelectSession={select} />);
  return { navigate, select };
}
function menu() {
  fireEvent.click(screen.getByRole('button', { name: 'Actions for Saved session' }));
}

describe('AgentsNavigationRail', () => {
  it('navigates to conversation after successfully creating a session', async () => {
    const { navigate } = setup();
    fireEvent.click(screen.getByRole('button', { name: 'New Session' }));
    expect(navigate).not.toHaveBeenCalled();
    await waitFor(() => expect(navigate).toHaveBeenCalledWith('conversation'));
  });

  it('deduplicates sessions and records selection only after backend success', async () => {
    const { select, navigate } = setup();
    expect(screen.queryByText('Durable duplicate')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /^Saved session/ }));
    expect(select).not.toHaveBeenCalled();
    await waitFor(() => expect(select).toHaveBeenCalledWith('saved'));
    expect(navigate).toHaveBeenCalledWith('conversation');
  });

  it('does not record rejected selection or failed new-session navigation', async () => {
    mocks.switchSession.mockResolvedValue(false);
    mocks.newSession.mockResolvedValue(false);
    const { select, navigate } = setup();
    fireEvent.click(screen.getByRole('button', { name: /^Saved session/ }));
    await waitFor(() => expect((screen.getByRole('button', { name: 'New Session' }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'New Session' }));
    await waitFor(() => expect(mocks.newSession).toHaveBeenCalled());
    expect(select).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });

  it('searches and filters backend sessions without per-keystroke reads', async () => {
    setup();
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'saved' } });
    expect(screen.queryByRole('button', { name: 'Live session Running' })).toBeNull();
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'running' } });
    await waitFor(() => expect(screen.getByText('No matching sessions')).toBeTruthy());
    expect(mocks.refreshAgents).toHaveBeenCalledTimes(1);
  });

  it('filters attention from transcript errors and renders textual failure status', async () => {
    mocks.state.transcripts = { live: [{ id: 'failed', role: 'assistant', content: 'Error', status: 'error', timestamp: 0 }] };
    setup();
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'attention' } });
    expect(screen.getByText('Failed')).toBeTruthy();
    expect(screen.queryByRole('button', { name: /^Saved session/ })).toBeNull();
    await waitFor(() => expect(screen.queryByText('Loading session history…')).toBeNull());
  });

  it('renders authoritative dates as secondary text with full tooltips', async () => {
    setup();
    const row = screen.getByRole('button', { name: /^Saved session/ });
    const date = row.querySelector('time');
    expect(date?.dateTime).toBe(new Date(1000).toISOString());
    expect(date?.title).toContain('1970');
    await waitFor(() => expect(screen.queryByText('Loading session history…')).toBeNull());
  });

  it('renders approval status for transcript tools awaiting a decision', async () => {
    mocks.state.transcripts = { live: [{ id: 'approval', role: 'assistant', content: '',
      status: 'complete', timestamp: 0, toolCalls: [{ id: 'tool', toolName: 'bash', argsSummary: '',
        status: 'needs_approval', startedAt: 0 }] }] };
    setup();
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'attention' } });
    expect(screen.getByText('Needs approval')).toBeTruthy();
    await waitFor(() => expect(screen.queryByText('Loading session history…')).toBeNull());
  });

  it('shows history loading only while the real snapshot request is pending', async () => {
    const original = mocks.state.resumableSessions;
    const originalLive = mocks.state.sessions;
    mocks.state.resumableSessions = [];
    mocks.state.sessions = [];
    let finish!: () => void;
    mocks.refreshAgents.mockReturnValue(new Promise<void>((resolve) => { finish = resolve; }));
    try {
      setup();
      expect(screen.getByText('Loading session history…')).toBeTruthy();
      expect(screen.queryByText('No sessions yet')).toBeNull();
      finish();
      await waitFor(() => expect(screen.getByText('No sessions yet')).toBeTruthy());
      expect(screen.queryByText('Loading session history…')).toBeNull();
    } finally {
      mocks.state.resumableSessions = original;
      mocks.state.sessions = originalLive;
    }
  });

  it('does not present history failures as an empty history', async () => {
    mocks.refreshAgents.mockRejectedValue(new Error('Snapshot unavailable'));
    setup();
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Snapshot unavailable'));
    expect(screen.queryByText('Loading session history…')).toBeNull();
    expect(screen.queryByText('No sessions yet')).toBeNull();
  });

  it('opens the targeted session and workspace in a new window', async () => {
    setup(); menu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Open in Agents window' }));
    await waitFor(() => expect(mocks.open).toHaveBeenCalledWith({ sessionId: 'saved', workspaceId: 'workspace' }));
  });

  it('renames with explicit target context and refreshes durable history', async () => {
    setup(); menu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename session' }));
    fireEvent.change(screen.getByLabelText('Session title'), { target: { value: 'Renamed' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(mocks.rename).toHaveBeenCalledWith('saved', 'Renamed', {
      workspace_id: 'workspace', core_instance_id: 'core', session_id: 'saved',
    }));
    expect(mocks.refresh).toHaveBeenCalled();
  });

  it('shows rename errors without refreshing or closing the rename form', async () => {
    mocks.rename.mockRejectedValue(new Error('Rename denied'));
    setup(); menu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename session' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('Rename denied'));
    expect(screen.getByRole('form', { name: 'Rename session' })).toBeTruthy();
    expect(mocks.refresh).not.toHaveBeenCalled();
  });

  it('targets pin, copy and show-main actions to the selected menu session', async () => {
    setup(); menu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Unpin session' }));
    expect(mocks.pin).toHaveBeenCalledWith('saved');
    menu(); fireEvent.click(screen.getByRole('menuitem', { name: 'Copy session ID' }));
    await waitFor(() => expect(mocks.copy).toHaveBeenCalledWith('saved'));
    await waitFor(() => expect((screen.getByRole('button', { name: 'Actions for Saved session' }) as HTMLButtonElement).disabled).toBe(false));
    menu(); fireEvent.click(screen.getByRole('menuitem', { name: 'Show in main window' }));
    await waitFor(() => expect(mocks.show).toHaveBeenCalledWith('saved', expect.objectContaining({ session_id: 'saved' })));
  });

  it('confirms durable deletion and blocks running deletion', async () => {
    setup(); menu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete session' }));
    expect(mocks.deleteSession).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    await waitFor(() => expect(mocks.deleteSession).toHaveBeenCalledWith('saved'));
    await waitFor(() => expect((screen.getByRole('button', { name: 'Actions for Live session' }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: 'Actions for Live session' }));
    expect((screen.getByRole('menuitem', { name: 'Delete session' }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('main-window session context menu', () => {
  it('always offers open-in-Agents with a default bridge action', async () => {
    const close = vi.fn();
    render(<AgentSessionContextMenu session={{ id: 'background', title: 'Background' }}
      x={0} y={0} isPinned={false} canDelete onTogglePin={vi.fn()} onDelete={vi.fn()} onClose={close} />);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Open in Agents window' }));
    await waitFor(() => expect(mocks.open).toHaveBeenCalledWith({ sessionId: 'background', workspaceId: 'workspace' }));
    expect(close).toHaveBeenCalled();
  });

  it('keeps default open failures visible', async () => {
    mocks.open.mockRejectedValue(new Error('Window unavailable'));
    const close = vi.fn();
    render(<AgentSessionContextMenu session={{ id: 'background', title: 'Background' }}
      x={0} y={0} isPinned={false} canDelete onTogglePin={vi.fn()} onDelete={vi.fn()} onClose={close} />);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Open in Agents window' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('Window unavailable'));
    expect(close).not.toHaveBeenCalled();
  });
});
