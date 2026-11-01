import { beforeEach, describe, expect, it, vi } from 'vitest';

const transport = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: transport.invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: transport.listen }));

import {
  agentsReadFilePreview,
  agentsReadGitDiff,
  agentsWindowReady,
  getAgentsSnapshot,
  openAgentsWindow,
  queueAgentPrompt,
  renameAgentSession,
  showSessionInMainWindow,
  subscribeAgentsNavigation,
  updateAgentPreferences,
  type AgentRequestDto,
  type EnvelopeContext,
} from './bridge';

const context: EnvelopeContext = {
  core_instance_id: 'core-1',
  workspace_id: 'workspace-1',
  session_id: 'other-window-selection',
};

beforeEach(() => {
  vi.clearAllMocks();
  transport.invoke.mockResolvedValue(undefined);
  transport.listen.mockResolvedValue(() => {});
});

describe('Agents window bridge', () => {
  it('uses a dedicated native operation and passes no draft or prompt in the route', async () => {
    await openAgentsWindow({ sessionId: 'parent', childId: 'child', workspaceId: 'workspace-1' });
    expect(transport.invoke).toHaveBeenCalledWith('open_agents_window', {
      sessionId: 'parent', childId: 'child', workspaceId: 'workspace-1',
    });
    await agentsWindowReady();
    expect(transport.invoke).toHaveBeenLastCalledWith('agents_window_ready');
  });

  it('addresses queue items using the captured session, not another window selection', async () => {
    const request: AgentRequestDto = {
      session_id: 'captured-session', prompt: 'Inspect these changes', model_id: 'model-1', mode: 'explore',
    };
    await queueAgentPrompt(request, context, 'stable-item');
    const envelope = transport.invoke.mock.calls[0][1].envelope;
    expect(envelope.session_id).toBe('captured-session');
    expect(envelope.workspace_id).toBe('workspace-1');
    expect(envelope.payload).toEqual({
      type: 'queue_agent', payload: { request_id: 'stable-item', request },
    });
  });

  it('targets rename and main-window navigation explicitly', async () => {
    await renameAgentSession('renamed-session', 'New title', context);
    expect(transport.invoke.mock.calls[0][1].envelope.session_id).toBe('renamed-session');
    await showSessionInMainWindow('parent', context);
    expect(transport.invoke).toHaveBeenLastCalledWith('show_session_in_main_window', {
      sessionId: 'parent', workspaceId: 'workspace-1',
    });
  });

  it('loads a complete runtime snapshot independently from navigation', async () => {
    await getAgentsSnapshot();
    expect(transport.invoke).toHaveBeenCalledWith('get_agents_snapshot', { protocolVersion: 1 });
    const handler = vi.fn();
    await subscribeAgentsNavigation(handler);
    const [name, listener] = transport.listen.mock.calls[0];
    expect(name).toBe('selora://agents-navigate');
    listener({ payload: { workspace_id: 'workspace-1', session_id: 'parent' } });
    expect(handler).toHaveBeenCalledWith({ workspace_id: 'workspace-1', session_id: 'parent' });
  });

  it('keeps read-only previews separate from main editor buffer activation', async () => {
    await agentsReadFilePreview('workspace-1', 'src/main.rs');
    await agentsReadGitDiff('workspace-1', 'src/main.rs', 'index');
    expect(transport.invoke.mock.calls).toEqual([
      ['agents_read_file_preview', { workspaceId: 'workspace-1', path: 'src/main.rs' }],
      ['agents_read_git_diff', { workspaceId: 'workspace-1', path: 'src/main.rs', baseline: 'index' }],
    ]);
  });

  it('sends field patches to the authoritative preference owner', async () => {
    await updateAgentPreferences({ systemInstructions: 'Review changes before editing.' });
    expect(transport.invoke).toHaveBeenCalledWith('update_agent_preferences', {
      patch: { systemInstructions: 'Review changes before editing.' },
    });
  });
});
