import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { makeBootstrap } from '../../../testing/fixtures';
import { AgentDock } from '../../../layout/AgentDock';
import { agentStore } from '../agent-store';

const PARENT = 'session-parent';
const OTHER = 'session-other';

function bootstrap() {
  return makeBootstrap({
    workspace: { id: 'ws', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
    chat: {
      sessions: [
        { session_id: PARENT, title: 'Parent', message_ids: [], active: true },
        { session_id: OTHER, title: 'Other', message_ids: [], active: false },
      ],
      messages: [],
      active_session_id: PARENT,
      active_request_id: null,
      active_turn_id: null,
      pending_approval: null,
    },
  });
}

beforeEach(() => {
  agentStore.reset();
  agentStore.hydrate(bootstrap());
  agentStore.handleSubagentActivity({
    kind: 'started',
    child_id: 'child-1',
    parent_session_id: PARENT,
    name: 'Parser audit',
    policy: 'read_only',
    mission: 'Audit the parser',
  });
});

describe('child transcript routing', () => {
  it('opens the child transcript from the results row and returns to the parent', () => {
    render(<AgentDock bootstrap={bootstrap()} />);
    expect(screen.queryByTestId('subagent-session-view')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Open session' }));

    const childView = screen.getByTestId('subagent-session-view');
    expect(childView.getAttribute('aria-label')).toContain('Parser audit');
    expect(agentStore.getSnapshot().activeSubagentId).toBe('child-1');

    fireEvent.click(screen.getByRole('button', { name: /Back to parent/ }));

    expect(screen.queryByTestId('subagent-session-view')).toBeNull();
    expect(agentStore.getSnapshot().activeSubagentId).toBeNull();
    expect(document.querySelector('.agent-composer')).not.toBeNull();
  });

  it('clears the open child when the parent session changes', () => {
    render(<AgentDock bootstrap={bootstrap()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Open session' }));
    expect(screen.getByTestId('subagent-session-view')).not.toBeNull();

    act(() => {
      agentStore.selectSessionLocally(OTHER);
    });

    expect(agentStore.getSnapshot().activeSubagentId).toBeNull();
    expect(screen.queryByTestId('subagent-session-view')).toBeNull();
  });

  it('keeps a cancelled child readable from the pet and shows the terminal note', () => {
    agentStore.handleSubagentCancelled({ child_id: 'child-1', parent_session_id: PARENT });
    render(<AgentDock bootstrap={bootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Open session' }));

    expect(screen.getByTestId('subagent-session-view')).not.toBeNull();
    expect(screen.getByText('Subagent was cancelled.')).not.toBeNull();
  });
});