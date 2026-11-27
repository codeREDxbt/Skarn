import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBootstrap } from '../testing/fixtures';
import { agentStore } from '../features/agent/agent-store';
import { clearAgentDraft, readAgentDraft, writeAgentDraft } from '../features/agent/draft-store';
import { AgentComposer } from '../features/agent/components/AgentComposer';

let workspaceId: string;
let fixtureNumber = 0;
const props = {
  inFlight: false, streamingActivityLevel: 0, permissionMode: 'ask_before_changes' as const,
  reasoningEffort: 'standard' as const, promptHistory: [],
};

beforeEach(() => {
  workspaceId = `draft-integration-${++fixtureNumber}`;
  agentStore.reset();
  agentStore.hydrate(makeBootstrap({
    workspace: { id: workspaceId, core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
    models: [{ id: 'model', display_name: 'Model', available: true }],
    active_model_id: 'model',
    chat: {
      sessions: [
        { session_id: 'first', title: 'First', active: true, message_ids: [] },
        { session_id: 'second', title: 'Second', active: false, message_ids: [] },
      ],
      messages: [], active_session_id: 'first', active_request_id: null,
      active_turn_id: null, pending_approval: null,
    },
  }));
});

afterEach(() => vi.restoreAllMocks());

describe('composer draft integration across surfaces', () => {
  it('restores each surface and session draft, attachments, and selection after remounts', () => {
    const main = render(<AgentComposer {...props} surface="main" />);
    const input = screen.getByRole('textbox') as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: 'Main first draft' } });
    input.setSelectionRange(2, 6);
    fireEvent.select(input);
    fireEvent.drop(main.container.querySelector('.agent-composer')!, { dataTransfer: {
      files: [{ path: 'C:/repo/src/main.ts' }],
    } });
    act(() => agentStore.selectSessionLocally('second'));
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('');
    expect(screen.queryByRole('button', { name: 'Remove attachment main.ts' })).toBeNull();
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Main second draft' } });
    act(() => agentStore.selectSessionLocally('first'));
    const restored = screen.getByRole('textbox') as HTMLTextAreaElement;
    expect(restored.value).toBe('Main first draft');
    expect([restored.selectionStart, restored.selectionEnd]).toEqual([2, 6]);
    expect(screen.getByRole('button', { name: 'Remove attachment main.ts' })).not.toBeNull();
    main.unmount();

    const agents = render(<AgentComposer {...props} surface="agents" />);
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('');
    expect(screen.queryByRole('button', { name: 'Remove attachment main.ts' })).toBeNull();
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Agents first draft' } });
    agents.unmount();
    render(<AgentComposer {...props} surface="main" />);
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('Main first draft');
    expect(readAgentDraft(workspaceId, 'first', 'agents').text).toBe('Agents first draft');
    expect(readAgentDraft(workspaceId, 'second', 'main').text).toBe('Main second draft');
  });

  it('scopes attachment events to workspace, session, and surface', () => {
    const main = render(<div aria-label="Main composer"><AgentComposer {...props} surface="main" /></div>);
    const agents = render(<div aria-label="Agents composer"><AgentComposer {...props} surface="agents" /></div>);
    const attach = (overrides: Record<string, string> = {}) => act(() => {
      window.dispatchEvent(new CustomEvent('skarn:attach-agent-file', { detail: {
        workspaceId, sessionId: 'first', surface: 'agents', path: 'C:/repo/src/agent.ts', ...overrides,
      } }));
    });
    attach({ workspaceId: 'other-workspace' });
    attach({ sessionId: 'second' });
    expect(screen.queryByRole('button', { name: 'Remove attachment agent.ts' })).toBeNull();
    attach();
    expect(within(main.container).queryByRole('button', { name: 'Remove attachment agent.ts' })).toBeNull();
    expect(within(agents.container).getByRole('button', { name: 'Remove attachment agent.ts' })).not.toBeNull();
  });

  it('keeps a rejected send draft and clears only the accepted surface', () => {
    const send = vi.spyOn(agentStore, 'sendUserMessage').mockReturnValue(false);
    writeAgentDraft(workspaceId, 'first', 'agents', { text: 'Other surface', attachments: [], revision: 0 });
    render(<AgentComposer {...props} surface="main" />);
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Keep until accepted' } });
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    expect(readAgentDraft(workspaceId, 'first', 'main').text).toBe('Keep until accepted');
    send.mockReturnValue(true);
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    expect(send).toHaveBeenCalledTimes(2);
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('');
    expect(readAgentDraft(workspaceId, 'first', 'agents').text).toBe('Other surface');
  });

  it('does not let acceptance of an older revision erase newer typing', () => {
    writeAgentDraft(workspaceId, 'first', 'main', { text: 'Submitted', attachments: [], revision: 0 });
    const submittedRevision = readAgentDraft(workspaceId, 'first', 'main').revision;
    writeAgentDraft(workspaceId, 'first', 'main', { text: 'Newer typing', attachments: ['src/new.ts'], revision: 0 });
    expect(clearAgentDraft(workspaceId, 'first', 'main', submittedRevision)).toBe(false);
    expect(readAgentDraft(workspaceId, 'first', 'main').text).toBe('Newer typing');
    expect(readAgentDraft(workspaceId, 'first', 'main').attachments).toEqual(['src/new.ts']);
  });
});
