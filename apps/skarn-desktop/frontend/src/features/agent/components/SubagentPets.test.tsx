import React, { useSyncExternalStore } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBootstrap } from '../../../testing/fixtures';
import { TitleBar } from '../../../layout/TitleBar';
import { agentStore } from '../agent-store';
import type { SubagentInfo } from '../types';
import {
  PET_WORLDS,
  PET_WORLD_LABEL,
  SubagentPets,
  petLabel,
  petWorldFor,
  petsForSession,
} from './SubagentPets';

const PARENT = 'session-parent';
const OTHER = 'session-other';

function hydrateSessions(): void {
  agentStore.hydrate(makeBootstrap({
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
  }));
}

function startChild(
  childId: string,
  parentSessionId: string,
  name: string | null = null,
  mission = '',
): void {
  agentStore.handleSubagentActivity({
    kind: 'started',
    child_id: childId,
    parent_session_id: parentSessionId,
    name,
    policy: 'read_only',
    mission,
  });
}

/** Mirrors TitleBar: one live snapshot of the store feeds the pet strip. */
function PetsFromStore({ sessionId }: { sessionId: string }) {
  const snapshot = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  return (
    <SubagentPets
      subagents={snapshot.subagents}
      activeSessionId={sessionId}
      activeSubagentId={snapshot.activeSubagentId}
      onSelectViewMode={() => undefined}
    />
  );
}

beforeEach(() => {
  agentStore.reset();
  hydrateSessions();
});

describe('pet derivation', () => {
  it('assigns each child a stable world and never invents a name', () => {
    expect(petWorldFor('child-1')).toBe(petWorldFor('child-1'));
    expect(PET_WORLDS).toContain(petWorldFor('child-1'));
    expect(petLabel({ id: 'child-1', name: '  Parser audit  ' })).toBe('Parser audit');
    expect(petLabel({ id: 'child-abcd', name: '   ' })).toBe('Subagent chil');
  });

  it('keeps only children of the requested parent session', () => {
    const children: SubagentInfo[] = [
      { id: 'a', name: 'A', model: 'Auto', status: 'working', parentSessionId: PARENT },
      { id: 'b', name: 'B', model: 'Auto', status: 'working', parentSessionId: OTHER },
      { id: 'c', name: 'C', model: 'Auto', status: 'working' },
    ];
    expect(petsForSession(children, PARENT).map((child) => child.id)).toEqual(['a']);
    expect(petsForSession(children, '')).toEqual([]);
  });
});

describe('pet strip wiring', () => {
  it('shows a companion without inventing a running child', () => {
    const { container } = render(<PetsFromStore sessionId={PARENT} />);
    const companion = screen.getByRole('button', { name: /Celestial companion Earth/ });
    expect(companion.getAttribute('data-world')).toBe('earth');
    expect(companion.getAttribute('data-status')).toBeNull();
    expect(companion.title).toContain('not a running subagent');
    expect(container.querySelector('svg.art-earth .pet-earth')).not.toBeNull();
    expect(agentStore.getSnapshot().subagents).toEqual([]);
    expect(screen.queryByRole('group', { name: 'Subagent pets' })).toBeNull();
  });

  it('opens Agent view from the companion without selecting a fake session', () => {
    const onSelectViewMode = vi.fn();
    render(<SubagentPets subagents={[]} activeSessionId={PARENT} onSelectViewMode={onSelectViewMode} />);
    fireEvent.click(screen.getByRole('button', { name: /Celestial companion Earth/ }));
    expect(onSelectViewMode).toHaveBeenCalledWith('agent');
    expect(agentStore.getSnapshot().activeSubagentId).toBeNull();
    expect(agentStore.getSnapshot().activeSessionId).toBe(PARENT);
  });

  it('does not turn a companion press into a title-bar drag', () => {
    const onPointerDown = vi.fn();
    render(
      <div onPointerDown={onPointerDown}>
        <SubagentPets subagents={[]} activeSessionId={PARENT} />
      </div>,
    );
    fireEvent.pointerDown(screen.getByRole('button', { name: /Celestial companion Earth/ }));
    expect(onPointerDown).not.toHaveBeenCalled();
  });

  it.each(PET_WORLDS)('renders distinct %s art for real children', (world) => {
    const id = Array.from({ length: 1000 }, (_, index) => `world-child-${index}`)
      .find((candidate) => petWorldFor(candidate) === world)!;
    expect(id).toBeDefined();
    const child: SubagentInfo = {
      id, name: `${world} task`, model: 'Auto', status: 'working', parentSessionId: PARENT,
    };
    render(<SubagentPets subagents={[child]} activeSessionId={PARENT} />);
    const pet = screen.getByRole('button', { name: new RegExp(PET_WORLD_LABEL[world]) });
    expect(pet.classList.contains(`world-${world}`)).toBe(true);
    expect(pet.querySelector(`svg.art-${world}`)?.childElementCount).toBeGreaterThan(0);
    expect(pet.getAttribute('data-status')).toBe('working');
    expect(screen.queryByRole('group', { name: 'Celestial Companion' })).toBeNull();
  });

  it('shows a pet for the started child and ignores another parent child', () => {
    startChild('child-1', PARENT, null, 'Audit the parser');
    startChild('child-2', OTHER, 'Other task');
    render(<PetsFromStore sessionId={PARENT} />);

    const pets = screen.getAllByRole('button');
    expect(pets).toHaveLength(1);
    expect(pets[0]!.getAttribute('data-status')).toBe('working');
    expect(pets[0]!.getAttribute('data-world')).toBe(petWorldFor('child-1'));
    expect(pets[0].getAttribute('aria-label')).toContain('Audit the parser');
  });

  it('switches the visible pets with the parent session', () => {
    startChild('child-1', PARENT, 'Parent task');
    startChild('child-2', OTHER, 'Other task');
    const { rerender } = render(<PetsFromStore sessionId={PARENT} />);
    expect(screen.getByRole('button').getAttribute('aria-label')).toContain('Parent task');

    rerender(<PetsFromStore sessionId={OTHER} />);
    expect(screen.getByRole('button').getAttribute('aria-label')).toContain('Other task');
  });

  it('opens the child transcript from a pet click and marks it active', () => {
    startChild('child-1', PARENT, 'Parser audit');
    render(<PetsFromStore sessionId={PARENT} />);

    const pet = screen.getByRole('button', { name: /Parser audit/ });
    fireEvent.click(pet);

    const snapshot = agentStore.getSnapshot();
    expect(snapshot.activeSubagentId).toBe('child-1');
    expect(snapshot.subagents[0]?.unread).toBe(false);
    expect(pet.getAttribute('aria-pressed')).toBe('true');
  });

  it('asks the shell for the agent view when a pet is clicked', () => {
    startChild('child-1', PARENT, 'Parser audit');
    const onSelectViewMode = vi.fn();
    const snapshot = agentStore.getSnapshot();
    render(
      <SubagentPets
        subagents={snapshot.subagents}
        activeSessionId={PARENT}
        onSelectViewMode={onSelectViewMode}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /Parser audit/ }));
    expect(onSelectViewMode).toHaveBeenCalledWith('agent');
  });

  it.each(['Enter', ' '])('opens the actual child once with the %s key', (key) => {
    startChild('child-1', PARENT, 'Parser audit');
    const onSelectViewMode = vi.fn();
    render(<SubagentPets subagents={agentStore.getSnapshot().subagents} activeSessionId={PARENT} onSelectViewMode={onSelectViewMode} />);
    fireEvent.keyDown(screen.getByRole('button', { name: /Parser audit/ }), { key });
    expect(onSelectViewMode).toHaveBeenCalledTimes(1);
    expect(agentStore.getSnapshot().activeSubagentId).toBe('child-1');
    expect(agentStore.getSnapshot().activeSessionId).toBe(PARENT);
  });

  it('marks a cancelled child and keeps its pet', () => {
    startChild('child-1', PARENT, 'Parser audit');
    agentStore.handleSubagentCancelled({ child_id: 'child-1', parent_session_id: PARENT });
    render(<PetsFromStore sessionId={PARENT} />);

    const pet = screen.getByRole('button', { name: /cancelled/ });
    expect(pet.getAttribute('data-status')).toBe('cancelled');
    expect(pet.className).toContain('is-cancelled');
    expect(pet.querySelector('.subagent-pet-mark.mark-cancelled')).not.toBeNull();
  });

  it('marks a failed child as failed, never as done', () => {
    startChild('child-1', PARENT, 'Parser audit');
    agentStore.handleSubagentCompleted({
      child_id: 'child-1',
      parent_session_id: PARENT,
      policy: 'read_only',
      ok: false,
      error: 'tool exploded',
      final_message: '',
    });
    render(<PetsFromStore sessionId={PARENT} />);

    const pet = screen.getByRole('button', { name: /failed/ });
    expect(pet.getAttribute('data-status')).toBe('failed');
    expect(pet.querySelector('.subagent-pet-mark.mark-failed')).not.toBeNull();
  });

  it('marks a completed child as done', () => {
    startChild('child-1', PARENT, 'Parser audit');
    agentStore.handleSubagentCompleted({
      child_id: 'child-1',
      parent_session_id: PARENT,
      policy: 'read_only',
      ok: true,
      final_message: 'All clean.',
    });
    render(<PetsFromStore sessionId={PARENT} />);

    const pet = screen.getByRole('button', { name: /done/ });
    expect(pet.getAttribute('data-status')).toBe('done');
    expect(pet.querySelector('.subagent-pet-mark.mark-done')).not.toBeNull();
  });

  it('drops pets when the parent session is deleted', () => {
    startChild('child-1', PARENT, 'Parser audit');
    agentStore.deleteSession(PARENT);
    render(<PetsFromStore sessionId={PARENT} />);

    expect(screen.queryByRole('button', { name: /Parser audit/ })).toBeNull();
    expect(screen.getByRole('button', { name: /Celestial companion Earth/ })).toBeDefined();
    expect(agentStore.getSnapshot().subagents).toEqual([]);
  });

  it('renders store children inside the title bar and forwards the view switch', () => {
    startChild('child-1', PARENT, 'Parser audit');
    const onSelectViewMode = vi.fn();
    const { container } = render(
      <TitleBar
        commands={[]}
        onRunCommand={() => undefined}
        viewMode="split"
        onSelectViewMode={onSelectViewMode}
      />,
    );

    const strip = container.querySelector('.titlebar-center .subagent-pets');
    expect(strip).not.toBeNull();
    expect(strip!.querySelectorAll('button')).toHaveLength(1);

    fireEvent.click(screen.getByRole('button', { name: /Parser audit/ }));
    expect(onSelectViewMode).toHaveBeenCalledWith('agent');
    expect(agentStore.getSnapshot().activeSubagentId).toBe('child-1');
  });
});
