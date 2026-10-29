import { act, fireEvent, render, screen, within } from '@testing-library/react';
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
        { session_id: SECOND, title: 'Second', active: false, message_ids: [], todos: [
          { id: 'second-task', text: 'Review the other session', status: 'in_progress' },
        ] },
      ],
      messages: [{ message_id: 'reply', session_id: FIRST, role: 'assistant', text: 'Chat stays readable.' }],
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

describe('compact in-feed tasks', () => {
  it('defaults to one collapsed summary without a duplicate todo spine', () => {
    const { container } = render(<AgentDock />);
    const toggle = screen.getByRole('button', { name: /Tasks \(1\/3\).*View tasks/ });

    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(within(toggle).getByText('Fix the compact task layout')).not.toBeNull();
    expect(toggle.title).toBe('Current: Fix the compact task layout');
    expect(container.querySelector('.zcode-infeed-expanded-body')).toBeNull();
    expect(container.querySelector('.agent-todo-spine')).toBeNull();
    expect(screen.queryByText('Run regression tests')).toBeNull();
    expect(screen.getByText('Chat stays readable.')).not.toBeNull();
    expect(container.querySelector('.agent-composer')).not.toBeNull();
  });

  it('expands and collapses task details without remounting the chat or composer', () => {
    const { container } = render(<AgentDock />);
    const feed = container.querySelector('.agent-execution-feed');
    const composer = container.querySelector('.agent-composer');
    const toggle = screen.getByRole('button', { name: /Tasks \(1\/3\)/ });

    fireEvent.click(toggle);

    const body = document.getElementById(toggle.getAttribute('aria-controls')!);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(toggle.closest('.zcode-infeed-progress-wrap')?.classList.contains('is-expanded')).toBe(true);
    expect(body).not.toBeNull();
    expect(within(body!).getByText('Run regression tests')).not.toBeNull();
    fireEvent.click(within(body!).getByRole('button', { name: /1 completed/ }));
    expect(within(body!).getByText('Read the code')).not.toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /Tasks \(1\/3\).*Collapse/ }));

    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(toggle.closest('.zcode-infeed-progress-wrap')?.classList.contains('is-expanded')).toBe(false);
    expect(document.getElementById(toggle.getAttribute('aria-controls')!)).toBeNull();
    expect(container.querySelector('.agent-execution-feed')).toBe(feed);
    expect(container.querySelector('.agent-composer')).toBe(composer);
  });

  it('updates the current task and count from authoritative todo events while collapsed', () => {
    render(<AgentDock />);

    act(() => agentStore.applySessionTodos({
      session_id: FIRST,
      todos: TODOS.map((todo) => ({
        ...todo,
        status: todo.id === 'next' ? 'in_progress' : 'completed',
      })),
    }));

    const toggle = screen.getByRole('button', { name: /Tasks \(2\/3\).*View tasks/ });
    expect(toggle.title).toBe('Current: Run regression tests');
    expect(within(toggle).getByText('Run regression tests')).not.toBeNull();
    expect(screen.queryByText('Fix the compact task layout')).toBeNull();
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
  });

  it.each(['pending', 'completed'] as const)('does not invent a current task for an all-%s list', (status) => {
    agentStore.applySessionTodos({ session_id: FIRST, todos: [{ id: 'only', text: 'Stored task', status }] });
    const { container } = render(<AgentDock />);
    const toggle = screen.getByRole('button', { name: /Tasks \(.*View tasks/ });

    expect(toggle.hasAttribute('title')).toBe(false);
    expect(container.querySelector('.infeed-current-task')).toBeNull();
    expect(container.querySelector('.infeed-bullet')).toBeNull();
    expect(screen.queryByText('Stored task')).toBeNull();
  });

  it('collapses on session switches and shows only the selected session summary', () => {
    render(<AgentDock />);
    fireEvent.click(screen.getByRole('button', { name: /Tasks \(1\/3\)/ }));

    act(() => agentStore.selectSessionLocally(SECOND));

    const toggle = screen.getByRole('button', { name: /Tasks \(0\/1\).*View tasks/ });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(toggle.title).toBe('Current: Review the other session');
    expect(screen.queryByText('Fix the compact task layout')).toBeNull();
    expect(document.getElementById(toggle.getAttribute('aria-controls')!)).toBeNull();

    act(() => agentStore.selectSessionLocally(FIRST));

    expect(screen.getByRole('button', { name: /Tasks \(1\/3\).*View tasks/ }).getAttribute('aria-expanded')).toBe('false');
  });

  it('omits task chrome when the selected session has no todos', () => {
    agentStore.hydrate(bootstrap([]));
    const { container } = render(<AgentDock />);

    expect(container.querySelector('.zcode-infeed-progress-wrap')).toBeNull();
    expect(container.querySelector('.agent-todo-spine')).toBeNull();
    expect(screen.getByText('Chat stays readable.')).not.toBeNull();
  });

  it('keeps the full plan available independently of collapsed chat tasks', () => {
    render(<AgentDock />);

    fireEvent.click(screen.getByRole('button', { name: 'Plan (1/3)' }));

    expect(screen.getByText('Plan Execution')).not.toBeNull();
    expect(screen.getByText('Run regression tests')).not.toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Chat' }));

    expect(screen.getByRole('button', { name: /Tasks \(1\/3\).*View tasks/ }).getAttribute('aria-expanded')).toBe('false');
    expect(screen.getByText('Chat stays readable.')).not.toBeNull();
  });

  it('lets task and child disclosures open together without remounting the conversation', () => {
    agentStore.handleSubagentActivity({
      kind: 'started', child_id: 'layout-child', parent_session_id: FIRST,
      name: 'Layout audit', policy: 'read_only', mission: 'Inspect the compact layout',
    });
    const { container } = render(<AgentDock />);
    const feed = container.querySelector('.agent-execution-feed');
    const composer = container.querySelector('.agent-composer');
    const children = screen.getByRole('button', { name: 'Subagent sessions (1)' });
    const tasks = screen.getByRole('button', { name: /Tasks \(1\/3\)/ });

    fireEvent.click(children);
    fireEvent.click(tasks);

    expect(children.getAttribute('aria-expanded')).toBe('true');
    expect(tasks.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByRole('button', { name: 'Open session' })).not.toBeNull();
    expect(screen.getByText('Run regression tests')).not.toBeNull();
    expect(container.querySelector('.agent-execution-feed')).toBe(feed);
    expect(container.querySelector('.agent-composer')).toBe(composer);

    fireEvent.click(children);

    expect(tasks.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByText('Chat stays readable.')).not.toBeNull();
  });
});
