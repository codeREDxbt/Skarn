import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import type { TodoDto } from '../bridge/bridge';
import { agentStore } from '../features/agent/agent-store';
import { makeBootstrap } from '../testing/fixtures';
import { AgentDock } from './AgentDock';

const FIRST = 'session-first';
const SECOND = 'session-second';
const TODOS: TodoDto[] = [
  { id: 'done', text: 'Read the code', status: 'completed' },
  { id: 'current', text: 'Fix the compact task layout', status: 'in_progress' },
  { id: 'next', text: 'Run regression tests', status: 'pending' },
];

function bootstrap(todos = TODOS) {
  return makeBootstrap({
    chat: {
      sessions: [
        { session_id: FIRST, title: 'First', active: true, message_ids: ['reply'], todos },
        {
          session_id: SECOND,
          title: 'Second',
          active: false,
          message_ids: [],
          todos: [{ id: 'second-task', text: 'Review the other session', status: 'in_progress' }],
        },
      ],
      messages: [
        { message_id: 'reply', session_id: FIRST, role: 'assistant', text: 'Chat stays readable.' },
      ],
      active_session_id: FIRST,
      active_request_id: null,
      active_turn_id: null,
      pending_approval: null,
    },
  });
}

beforeEach(() => {
  agentStore.reset();
  agentStore.hydrate(bootstrap());
});

describe('canonical Plan/Tasks view and header cleanup', () => {
  it('omits duplicate floating plan toggle, infeed progress bar, and hidden session rail', () => {
    const { container } = render(<AgentDock />);

    expect(container.querySelector('.zcode-infeed-progress-wrap')).toBeNull();
    expect(container.querySelector('.agent-floating-plan-toggle')).toBeNull();
    expect(container.querySelector('.composer-mode-row')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Session history' })).toBeNull();
    expect(screen.getByText('Chat stays readable.')).not.toBeNull();
    expect(container.querySelector('.agent-composer')).not.toBeNull();
  });

  it('switches cleanly to the single canonical Plan view and back without losing chat or composer', () => {
    const { container } = render(<AgentDock />);
    const composer = container.querySelector('.agent-composer');

    fireEvent.click(screen.getByRole('button', { name: 'Plan (1/3)' }));

    expect(screen.getByText('Plan Execution')).not.toBeNull();
    expect(screen.getByText('Run regression tests')).not.toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Chat' }));

    expect(screen.getByText('Chat stays readable.')).not.toBeNull();
    expect(container.querySelector('.agent-composer')).toBe(composer);
  });

  it('updates the canonical Plan count from authoritative todo events', () => {
    render(<AgentDock />);

    act(() =>
      agentStore.applySessionTodos({
        session_id: FIRST,
        todos: TODOS.map((todo) => ({
          ...todo,
          status: todo.id === 'next' ? 'in_progress' : 'completed',
        })),
      }),
    );

    expect(screen.getByRole('button', { name: 'Plan (2/3)' })).not.toBeNull();
  });

  it('keeps canonical permission mode vocabulary without Ask/Agent/Plan row or Armed synonyms', () => {
    const { container } = render(<AgentDock />);

    fireEvent.click(container.querySelector('.mode-pill')!);

    expect(screen.getByRole('menuitem', { name: /Read-only/ })).not.toBeNull();
    expect(screen.getByRole('menuitem', { name: /Ask before changes/ })).not.toBeNull();
    expect(screen.getByRole('menuitem', { name: /Auto-edit/ })).not.toBeNull();
    expect(screen.queryByText('Armed')).toBeNull();
    expect(screen.queryByText('Guarded')).toBeNull();
    expect(screen.queryByText('Full auto-apply')).toBeNull();
  });
});
