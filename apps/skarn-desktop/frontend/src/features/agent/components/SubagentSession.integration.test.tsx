import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBootstrap } from '../../../testing/fixtures';
import { AgentDock } from '../../../layout/AgentDock';
import { agentStore } from '../agent-store';
import { PET_WORLD_LABEL, petWorldFor } from './SubagentPets';

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

afterEach(() => vi.restoreAllMocks());

function renderExpandedDock() {
  const result = render(<AgentDock bootstrap={bootstrap()} />);
  fireEvent.click(screen.getByRole('button', { name: /Subagent sessions \(/ }));
  return result;
}

describe('child transcript routing', () => {
  it.each(['moon', 'earth', 'sun'] as const)('keeps the %s world visible across results and transcript routing', (world) => {
    const childId = Array.from({ length: 1000 }, (_, index) => `world-child-${index}`)
      .find((id) => petWorldFor(id) === world)!;
    expect(childId).toBeDefined();
    agentStore.handleSubagentActivity({
      kind: 'started', child_id: childId, parent_session_id: PARENT,
      name: 'World audit', policy: 'read_only', mission: 'Inspect a real child session',
    });
    renderExpandedDock();

    const row = screen.getByRole('button', { name: 'Expand result for World audit' }).closest('article')!;
    const resultWorld = row.querySelector('.subagent-result-world');
    expect(resultWorld?.getAttribute('data-world')).toBe(world);
    expect(resultWorld?.textContent).toBe(PET_WORLD_LABEL[world]);
    expect(resultWorld?.querySelector(`.subagent-pet-world.world-${world} svg.art-${world}`)).not.toBeNull();
    expect(resultWorld?.querySelector('.subagent-pet-world')?.getAttribute('aria-hidden')).toBe('true');
    expect(within(row).getByText('read_only')).not.toBeNull();
    expect(within(row).getByText('Running')).not.toBeNull();

    fireEvent.click(within(row).getByRole('button', { name: 'Open session' }));

    const childView = screen.getByTestId('subagent-session-view');
    const transcriptWorld = childView.querySelector('.subagent-session-world');
    expect(transcriptWorld?.getAttribute('data-world')).toBe(world);
    expect(transcriptWorld?.textContent).toBe(resultWorld?.textContent);
    expect(transcriptWorld?.querySelector(`.subagent-pet-world.world-${world} svg.art-${world}`)).not.toBeNull();
    expect(childView.getAttribute('aria-label')).toBe('Subagent session World audit');
    expect(within(childView).getByText('read_only')).not.toBeNull();
    expect(agentStore.getSnapshot().activeSubagentId).toBe(childId);

    fireEvent.click(within(childView).getByRole('button', { name: 'Back to parent (Parent)' }));

    expect(screen.queryByTestId('subagent-session-view')).toBeNull();
    expect(agentStore.getSnapshot().activeSubagentId).toBeNull();
    expect(row.querySelector('.subagent-result-world')?.getAttribute('data-world')).toBe(world);
  });

  it('opens the child transcript from the results row and returns to the parent', () => {
    renderExpandedDock();
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
    renderExpandedDock();
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
    renderExpandedDock();

    const world = petWorldFor('child-1');
    const row = screen.getByRole('button', { name: 'Expand result for Parser audit' }).closest('article')!;
    expect(within(row).getByText('Cancelled')).not.toBeNull();
    expect(row.querySelector('.subagent-result-world')?.getAttribute('data-world')).toBe(world);
    fireEvent.click(within(row).getByRole('button', { name: 'Open session' }));

    const childView = screen.getByTestId('subagent-session-view');
    expect(childView.querySelector('.subagent-session-world')?.getAttribute('data-world')).toBe(world);
    expect(within(childView).getByText('Cancelled')).not.toBeNull();
    expect(screen.getByText('Subagent was cancelled.')).not.toBeNull();
  });

  it.each([true, false])('preserves the world and terminal status after completion (ok=%s)', (ok) => {
    renderExpandedDock();
    fireEvent.click(screen.getByRole('button', { name: 'Open session' }));
    const world = petWorldFor('child-1');

    act(() => {
      agentStore.handleSubagentCompleted({
        child_id: 'child-1', parent_session_id: PARENT, policy: 'read_only', ok,
        final_message: ok ? 'Parser audit complete.' : '', error: ok ? undefined : 'Parser audit failed.',
      });
    });

    const childView = screen.getByTestId('subagent-session-view');
    expect(within(childView).getByText(ok ? 'Finished' : 'Failed')).not.toBeNull();
    expect(within(childView).getByText(ok ? 'Parser audit complete.' : 'Parser audit failed.')).not.toBeNull();
    expect(childView.querySelector('.subagent-session-world')?.getAttribute('data-world')).toBe(world);
    expect(agentStore.getSnapshot().activeSubagentId).toBe('child-1');

    fireEvent.click(within(childView).getByRole('button', { name: /Back to parent/ }));

    const row = screen.getByRole('button', { name: 'Expand result for Parser audit' }).closest('article')!;
    expect(within(row).getByText(ok ? 'Succeeded' : 'Failed')).not.toBeNull();
    expect(row.querySelector('.subagent-result-world')?.getAttribute('data-world')).toBe(world);
    expect(screen.queryByTestId('subagent-session-view')).toBeNull();
  });
});

describe('compact child disclosure', () => {
  it('defaults to an accessible summary and exposes truthful rows only on request', () => {
    const { container } = render(<AgentDock bootstrap={bootstrap()} />);
    const composer = container.querySelector('.agent-composer');
    const summary = screen.getByRole('button', { name: 'Subagent sessions (1)' });

    expect(summary.getAttribute('aria-expanded')).toBe('false');
    expect(summary.textContent).toContain('1 running · 0 finished');
    expect(screen.queryByRole('button', { name: 'Open session' })).toBeNull();
    expect(screen.queryByRole('group', { name: 'Child session list' })).toBeNull();

    fireEvent.click(summary);

    expect(summary.getAttribute('aria-expanded')).toBe('true');
    const list = screen.getByRole('group', { name: 'Child session list' });
    expect(list.id).toBe(summary.getAttribute('aria-controls'));
    expect(within(list).getByText('Parser audit')).not.toBeNull();
    expect(within(list).getByText('Running')).not.toBeNull();
    expect(within(list).getByText('read_only')).not.toBeNull();
    expect(within(list).getByRole('button', { name: 'Open session' })).not.toBeNull();

    fireEvent.click(summary);

    expect(summary.getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByRole('button', { name: 'Open session' })).toBeNull();
    expect(container.querySelector('.agent-composer')).toBe(composer);
  });

  it('resets the disclosure when the selected parent changes', () => {
    renderExpandedDock();
    act(() => agentStore.selectSessionLocally(OTHER));
    expect(screen.queryByRole('button', { name: /Subagent sessions \(/ })).toBeNull();

    act(() => agentStore.selectSessionLocally(PARENT));

    expect(screen.getByRole('button', { name: 'Subagent sessions (1)' }).getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByRole('button', { name: 'Open session' })).toBeNull();
  });

  it('opens the correct disclosure and focuses the revealed child from a toast', async () => {
    for (let index = 2; index <= 6; index++) {
      agentStore.handleSubagentActivity({
        kind: 'started', child_id: `child-${index}`, parent_session_id: PARENT,
        name: `Child ${index}`, policy: 'read_only', mission: `Inspect task ${index}`,
      });
    }
    agentStore.handleSubagentCompleted({
      child_id: 'child-6', parent_session_id: PARENT, policy: 'read_only', ok: true,
      final_message: 'Last child result is readable.',
    });
    const scroll = vi.spyOn(HTMLElement.prototype, 'scrollIntoView').mockImplementation(() => {});
    render(<AgentDock bootstrap={bootstrap()} />);
    const summary = screen.getByRole('button', { name: 'Subagent sessions (6)' });
    expect(summary.getAttribute('aria-expanded')).toBe('false');
    expect(summary.textContent).toContain('5 running · 1 finished · 1 unread');
    fireEvent.click(screen.getByRole('button', { name: 'Plan (0/0)' }));

    await act(async () => {
      expect(await agentStore.revealSubagent(PARENT, 'child-6')).toBe(true);
    });

    const revealed = screen.getByRole('button', { name: 'Collapse result for Child 6' });
    expect(summary.getAttribute('aria-expanded')).toBe('true');
    expect(document.activeElement).toBe(revealed);
    expect(scroll).toHaveBeenCalledWith({ block: 'nearest' });
    expect(screen.getByText('Last child result is readable.')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Expand result for Child 2' }).getAttribute('aria-expanded')).toBe('false');
    expect(screen.getByRole('button', { name: 'Chat' }).getAttribute('aria-pressed')).toBe('true');

    fireEvent.click(summary);
    expect(agentStore.getSnapshot().revealedSubagentId).toBeNull();
    expect(screen.queryByText('Last child result is readable.')).toBeNull();

    await act(async () => { await agentStore.revealSubagent(PARENT, 'child-6'); });

    expect(screen.getByRole('button', { name: 'Collapse result for Child 6' })).toBe(document.activeElement);
  });
});
