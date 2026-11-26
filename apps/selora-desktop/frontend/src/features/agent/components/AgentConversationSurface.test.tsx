import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBootstrap } from '../../../testing/fixtures';
import { agentStore } from '../agent-store';
import { clearAgentDraft, readAgentDraft, writeAgentDraft } from '../draft-store';
import { AgentConversationSurface } from './AgentConversationSurface';

const workspaceId = 'surface-workspace';
const sessionId = 'surface-session';
const bootstrap = () => makeBootstrap({
  workspace: { id: workspaceId, root: 'C:/repo', display_name: 'repo', core_instance_id: 'core-0001' },
  models: [{ id: 'test-model', display_name: 'Test model', available: true }],
  active_model_id: 'test-model',
  chat: {
    sessions: [{ session_id: sessionId, title: 'Shared session', active: false, message_ids: [] }],
    messages: [], active_session_id: sessionId, active_request_id: null, active_turn_id: null, pending_approval: null,
  },
});

beforeEach(() => {
  agentStore.reset();
  agentStore.hydrate(bootstrap());
  clearAgentDraft(workspaceId, sessionId, 'main');
  clearAgentDraft(workspaceId, sessionId, 'agents');
});
afterEach(() => vi.restoreAllMocks());

describe('shared conversation surface', () => {
  it('keeps the same composer when welcome becomes an active conversation', () => {
    const { container } = render(<AgentConversationSurface presentation="agents" />);
    const input = screen.getByRole('textbox', { name: 'Composer prompt input' });
    fireEvent.change(input, { target: { value: 'Keep my draft' } });
    act(() => agentStore.hydrate({ ...bootstrap(), chat: {
      ...bootstrap().chat,
      messages: [{ message_id: 'answer', session_id: sessionId, role: 'assistant', text: 'Shared transcript' }],
    } }));
    expect(screen.getByRole('textbox', { name: 'Composer prompt input' })).toBe(input);
    expect((input as HTMLTextAreaElement).value).toBe('Keep my draft');
    expect(container.querySelector('[data-conversation-state="active"]')).not.toBeNull();
    expect(screen.queryByRole('button', { name: 'Open Agents window' })).toBeNull();
  });

  it('persists changes immediately and isolates surfaces across remounts', () => {
    const view = render(<AgentConversationSurface presentation="agents" />);
    fireEvent.change(screen.getByRole('textbox', { name: 'Composer prompt input' }), { target: { value: 'Agents draft' } });
    expect(readAgentDraft(workspaceId, sessionId, 'agents').text).toBe('Agents draft');
    expect(readAgentDraft(workspaceId, sessionId, 'main').text).toBe('');
    view.unmount();
    render(<AgentConversationSurface presentation="agents" />);
    expect((screen.getByRole('textbox', { name: 'Composer prompt input' }) as HTMLTextAreaElement).value).toBe('Agents draft');
  });

  it('retains rejected submissions and clears accepted submissions', () => {
    const send = vi.spyOn(agentStore, 'sendUserMessage').mockReturnValue(false);
    render(<AgentConversationSurface presentation="agents" />);
    const input = screen.getByRole('textbox', { name: 'Composer prompt input' });
    fireEvent.change(input, { target: { value: 'Do work' } });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect((input as HTMLTextAreaElement).value).toBe('Do work');
    expect(readAgentDraft(workspaceId, sessionId, 'agents').text).toBe('Do work');
    send.mockReturnValue(true);
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    expect(send).toHaveBeenLastCalledWith('Do work', false, undefined);
    expect(readAgentDraft(workspaceId, sessionId, 'agents').text).toBe('');
  });

  it('routes explicit draft restore to only the requested surface and session', () => {
    const main = render(<AgentConversationSurface />);
    const agents = render(<AgentConversationSurface presentation="agents" />);
    act(() => window.dispatchEvent(new CustomEvent('selora:set-composer-draft', {
      detail: { text: 'Agents only', sessionId, surface: 'agents' },
    })));
    expect((within(main.container).getByRole('textbox', { name: 'Composer prompt input' }) as HTMLTextAreaElement).value).toBe('');
    expect((within(agents.container).getByRole('textbox', { name: 'Composer prompt input' }) as HTMLTextAreaElement).value).toBe('Agents only');
  });

  it('validates attachment events, deduplicates paths, caps files, and emits preview intent', () => {
    render(<AgentConversationSurface presentation="agents" />);
    const attach = (path: string, targetWorkspace = workspaceId) => act(() => window.dispatchEvent(new CustomEvent('selora:attach-agent-file', {
      detail: { path, sessionId, surface: 'agents', workspaceId: targetWorkspace },
    })));
    attach('C:/outside/secret.txt');
    attach('src/wrong.ts', 'other-workspace');
    attach('src/first.ts');
    attach('src/first.ts');
    for (let index = 0; index < 10; index++) attach(`src/file-${index}.ts`);
    expect(readAgentDraft(workspaceId, sessionId, 'agents').attachments).toHaveLength(8);
    expect(screen.queryByText('secret.txt')).toBeNull();
    const preview = vi.fn();
    window.addEventListener('selora:preview-agent-file', preview);
    fireEvent.click(screen.getByRole('button', { name: 'Preview attachment first.ts' }));
    expect(preview).toHaveBeenCalledOnce();
    window.removeEventListener('selora:preview-agent-file', preview);
  });

  it('restores only safe persisted attachment paths', () => {
    writeAgentDraft(workspaceId, sessionId, 'agents', {
      text: 'Restored', attachments: ['../secret', 'src/valid.ts'], revision: 0,
    });
    render(<AgentConversationSurface presentation="agents" />);
    expect(screen.queryByRole('button', { name: 'Remove attachment secret' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Remove attachment valid.ts' })).not.toBeNull();
  });

  it('surfaces native provider thinking modes from model metadata cleanly', () => {
    const nativeBootstrap = makeBootstrap({
      workspace: { id: workspaceId, root: 'C:/repo', display_name: 'repo', core_instance_id: 'core-0001' },
      models: [{
        id: 'openrouter/thinking-pro',
        display_name: 'Thinking Pro',
        available: true,
        tool_capability: 'supported',
        reasoning_capability: 'native',
        reasoning_modes: ['none', 'quick', 'max'],
      }, {
        id: 'openai/gpt-4o',
        display_name: 'GPT-4o',
        available: true,
        tool_capability: 'supported',
        reasoning_capability: 'unsupported',
        reasoning_modes: [],
      }],
      active_model_id: 'openrouter/thinking-pro',
      chat: {
        sessions: [{ session_id: sessionId, title: 'Shared session', active: false, message_ids: [] }],
        messages: [], active_session_id: sessionId, active_request_id: null, active_turn_id: null, pending_approval: null,
      },
    });
    agentStore.hydrate(nativeBootstrap);
    render(<AgentConversationSurface presentation="agents" />);

    fireEvent.click(screen.getByRole('button', { name: /Standard|Max|Quick|Off/i }));
    expect(screen.getByText('Native Thinking Mode')).not.toBeNull();
    expect(screen.getByText('Off')).not.toBeNull();
    expect(screen.queryByText('Balanced speed and analysis')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: /Thinking Pro/i }));
    expect(screen.getAllByText('Thinking').length).toBeGreaterThanOrEqual(1);
  });
});
