import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { openAgentsWindow } from '../bridge/bridge';
import { agentStore } from '../features/agent/agent-store';
import { AgentConversationSurface } from '../features/agent/components/AgentConversationSurface';
import { editorStore } from '../features/editor/editor-store';
import { layoutStore } from '../features/layout/layout-store';
import { AgentDock } from '../layout/AgentDock';
import { makeBootstrap } from '../testing/fixtures';

vi.mock('../bridge/bridge', async (original) => ({
  ...await original<typeof import('../bridge/bridge')>(),
  openAgentsWindow: vi.fn().mockResolvedValue(undefined),
}));

const snapshot = makeBootstrap({
  workspace: { id: 'sidebar-regression', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
  chat: {
    sessions: [{ session_id: 'parent', title: 'Parent conversation', active: true, message_ids: ['reply'],
      todos: [{ id: 'task', title: 'Review layout', status: 'in_progress' }] }],
    messages: [{ message_id: 'reply', session_id: 'parent', role: 'assistant', text: 'Existing main transcript' }],
    active_session_id: 'parent', active_request_id: null, active_turn_id: null, pending_approval: null,
  },
});

beforeEach(() => {
  agentStore.reset();
  agentStore.hydrate(snapshot);
  layoutStore.hydrate(snapshot);
  vi.mocked(openAgentsWindow).mockClear();
});
afterEach(() => vi.restoreAllMocks());

describe('main sidebar and detached conversation composition', () => {
  it('opens the selected conversation without changing main layout, editor, or transcript', () => {
    render(<AgentDock bootstrap={snapshot} />);
    const layout = layoutStore.getSnapshot();
    const editor = editorStore.getSnapshot();
    const transcript = agentStore.getSnapshot().messages;
    expect(screen.getByRole('button', { name: 'Editor view' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Split view' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Plan (0/1)' })).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Open Agents window' }));

    expect(openAgentsWindow).toHaveBeenCalledTimes(1);
    expect(openAgentsWindow).toHaveBeenCalledWith({
      sessionId: 'parent', childId: undefined, workspaceId: 'sidebar-regression',
    });
    expect(layoutStore.getSnapshot()).toBe(layout);
    expect(editorStore.getSnapshot()).toBe(editor);
    expect(agentStore.getSnapshot().messages).toBe(transcript);
    expect(screen.getByText('Existing main transcript')).not.toBeNull();
  });

  it('keeps main task controls and composer mounted across plan navigation', () => {
    render(<AgentDock bootstrap={snapshot} />);
    fireEvent.click(screen.getByRole('button', { name: /Tasks \(0\/1\).*View tasks/ }));
    expect(screen.getByText('Existing main transcript')).not.toBeNull();
    expect(screen.getByRole('textbox')).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Plan (0/1)' }));
    expect(screen.getByText('Plan Execution')).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Chat' }));
    expect(screen.getByRole('textbox')).not.toBeNull();
    expect(screen.getByText('Existing main transcript')).not.toBeNull();
  });

  it('renders the detached transcript without main-only navigation chrome', () => {
    render(<AgentConversationSurface bootstrap={snapshot} presentation="agents" />);
    expect(screen.getByText('Existing main transcript')).not.toBeNull();
    expect(screen.getByRole('textbox')).not.toBeNull();
    expect(screen.queryByRole('button', { name: 'Open Agents window' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Editor view' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Split view' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Session history' })).toBeNull();
    expect(screen.queryByRole('tablist')).toBeNull();
  });

  it('passes selected child identity to native navigation and keeps parent composer recoverable', () => {
    agentStore.handleSubagentActivity({ kind: 'started', child_id: 'child', parent_session_id: 'parent',
      name: 'Review child', policy: 'read_only', mission: 'Review sidebar behavior' });
    render(<AgentDock bootstrap={snapshot} />);
    act(() => { expect(agentStore.openSubagentSession('child')).toBe(true); });
    const child = screen.getByTestId('subagent-session-view');
    expect(within(child).getByText('Review sidebar behavior')).not.toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Open Agents window' }));
    expect(openAgentsWindow).toHaveBeenCalledWith({
      sessionId: 'parent', childId: 'child', workspaceId: 'sidebar-regression',
    });
    fireEvent.click(within(child).getByRole('button', { name: /Back to parent/ }));
    expect(screen.queryByTestId('subagent-session-view')).toBeNull();
    expect(screen.getByRole('textbox')).not.toBeNull();
    expect(screen.getByText('Existing main transcript')).not.toBeNull();
  });
});
