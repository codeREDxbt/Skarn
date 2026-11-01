import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentSessionLiveDto, AgentsSnapshotDto, EventEnvelope } from '../bridge/bridge';
import { makeBootstrap, makeEvent } from '../testing/fixtures';

const transport = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  replay: vi.fn(),
  listen: vi.fn(),
  ready: vi.fn().mockResolvedValue(undefined),
  agents: vi.fn(),
  native: false,
  switchSession: vi.fn().mockResolvedValue(undefined),
  send: vi.fn(),
}));

vi.mock('../bridge/bridge', async (original) => ({
  ...await original<typeof import('../bridge/bridge')>(),
  getBootstrapState: transport.bootstrap,
  getEventsAfter: transport.replay,
  listenProtocolEvent: transport.listen,
  markFrontendReady: transport.ready,
  getAgentsSnapshot: transport.agents,
  switchSession: transport.switchSession,
  sendAgent: transport.send,
  subscribeAgentPreferences: vi.fn().mockResolvedValue(() => {}),
  getAgentPreferences: vi.fn().mockResolvedValue({
    revision: 0, temperature: 0.3, systemInstructions: '', customModelId: '',
    selectedModel: '', reasoningEffort: 'standard',
  }),
}));
vi.mock('./surface', () => ({
  isNativeFrontend: () => transport.native,
  getFrontendSurface: () => ({ kind: 'main', id: 'main' }),
}));
vi.mock('../features/theme/theme', () => ({
  applyAppearance: vi.fn(), watchSystemTheme: () => () => {},
}));
vi.mock('../features/sound/sound-store', () => ({ playCue: vi.fn() }));

function toast(sequence: number, message: string) {
  return makeEvent(sequence, { type: 'toast', payload: { severity: 'info', message } });
}

function session(id: string, overrides: Partial<AgentSessionLiveDto> = {}): AgentSessionLiveDto {
  return {
    session_id: id, title: id, created_at: null, updated_at: null, model_id: null,
    mode: 'execute', durable: false, active_request_id: null, active_turn_id: null,
    state: null, messages: [], timeline: [], approvals: [], plan_phase: 'executing',
    plan: null, todos: [], queue: [], subagents: [], ...overrides,
  };
}

async function runtime() {
  // Separate module graphs approximate separate webview heaps, not native windows.
  // DOM and localStorage remain shared in happy-dom.
  vi.resetModules();
  const app = await import('./app-store');
  const { agentStore } = await import('../features/agent/agent-store');
  const { toastStore } = await import('../features/toasts/toast-store');
  return { app, agentStore, toastStore };
}

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  transport.bootstrap.mockReset().mockResolvedValue(makeBootstrap());
  transport.replay.mockReset().mockResolvedValue([]);
  transport.listen.mockReset().mockResolvedValue(vi.fn());
  transport.ready.mockClear();
  transport.agents.mockReset();
  transport.native = false;
  transport.switchSession.mockClear();
  transport.send.mockClear();
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe('isolated window runtime delivery', () => {
  it('refreshes authoritative queues and approvals without replaying queue execution or losing local selection', async () => {
    const window = await runtime();
    window.agentStore.hydrate(makeBootstrap({
      workspace: { id: 'ws', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
    }));
    const snapshot: AgentsSnapshotDto = {
      protocol_version: 1, core_instance_id: 'core-0001', workspace_id: 'ws',
      snapshot_sequence: 12, selected_session_id: 'first',
      sessions: [session('first', {
        active_request_id: 'request-first', active_turn_id: 'turn-first', state: 'awaiting_approval',
        timeline: [{ sequence: 10, event: { kind: 'tool_call_started', payload: {
          request_id: 'request-first', session_id: 'first', turn_id: 'turn-first',
          call_id: 'bash-first', tool_name: 'bash', args_json: '{"command":"echo hello"}',
        } } }],
        queue: [{ request_id: 'queued-first', created_at: 1, status: 'queued', error: null,
          request: { session_id: 'first', prompt: 'Queued first prompt', model_id: 'model', mode: 'execute' } }],
        approvals: [{ request_id: 'request-first', session_id: 'first', turn_id: 'turn-first',
          approval_id: 'approval-first', tool_name: 'bash', path_or_command: 'echo hello' }],
      }), session('second', {
        messages: [{ message_id: 'reply-second', session_id: 'second', role: 'assistant', text: 'Second transcript' }],
      })],
    };
    window.agentStore.hydrateAgents(snapshot);
    expect(window.agentStore.getSnapshot().promptQueue.map((entry) => entry.id)).toEqual(['queued-first']);
    expect(await window.agentStore.requestSwitchSession('second')).toBe(true);
    window.agentStore.hydrateAgents({ ...snapshot, snapshot_sequence: 13, selected_session_id: 'first' });
    expect(window.agentStore.getSnapshot().activeSessionId).toBe('second');
    expect(window.agentStore.getSnapshot().messages.map((entry) => entry.content)).toEqual(['Second transcript']);
    expect(await window.agentStore.requestSwitchSession('first')).toBe(true);
    const approvals = window.agentStore.getSnapshot().messages.flatMap((message) => message.toolCalls ?? [])
      .filter((call) => call.approvalId === 'approval-first');
    expect(approvals).toHaveLength(1);
    expect(window.agentStore.getSnapshot().promptQueue.map((entry) => entry.id)).toEqual(['queued-first']);
    window.agentStore.checkAndExecuteNextQueued();
    window.agentStore.checkAndExecuteNextQueued();
    await vi.advanceTimersByTimeAsync(1000);
    expect(transport.send).not.toHaveBeenCalled();
    expect(transport.switchSession).not.toHaveBeenCalled();
  });

  it('hydrates the full Agents snapshot and skips covered agent events without dropping global events', async () => {
    transport.native = true;
    transport.bootstrap.mockResolvedValue(makeBootstrap({
      workspace: { id: 'ws', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
    }));
    const snapshot: AgentsSnapshotDto = {
      protocol_version: 1, core_instance_id: 'core-0001', workspace_id: 'ws',
      snapshot_sequence: 4, selected_session_id: 'first',
      sessions: [session('first', {
        active_request_id: 'request-first', active_turn_id: 'turn-first', state: 'thinking',
        messages: [{ message_id: 'reply-first', session_id: 'first', role: 'assistant', text: 'Snapshot text' }],
      }), session('second')],
    };
    transport.agents.mockResolvedValue(snapshot);
    const delta = (sequence: number, text: string) => makeEvent(sequence, { type: 'chat_delta', payload: {
      request_id: 'request-first', session_id: 'first', turn_id: 'turn-first',
      message_id: 'reply-first', sequence, text,
    } });
    let deliver!: (event: EventEnvelope) => void;
    transport.listen.mockImplementation(async (listener) => { deliver = listener; return vi.fn(); });
    transport.replay.mockResolvedValue([
      delta(1, 'Old text'), toast(2, 'Historical global notification'),
      delta(3, 'Already projected'), delta(4, 'Already projected too'),
    ]);
    const window = await runtime();
    await window.app.startApp();
    expect(window.app.getAppSnapshot().phase).toBe('ready');
    expect(window.agentStore.getSnapshot().sessions.map((entry) => entry.id)).toEqual(['first', 'second']);
    expect(window.agentStore.getSnapshot().messages.map((entry) => entry.content)).toEqual(['Snapshot text']);
    deliver(delta(5, ' plus live text'));
    deliver(toast(6, 'Contiguous live notification'));
    expect(window.agentStore.getSnapshot().messages.map((entry) => entry.content)).toEqual(['Snapshot text plus live text']);
    expect(window.toastStore.getSnapshot().history.map((entry) => entry.message)).toEqual([
      'Contiguous live notification', 'Historical global notification',
    ]);
    expect(transport.agents).toHaveBeenCalledTimes(1);
    expect(transport.bootstrap).toHaveBeenCalledTimes(1);
  });

  it('applies replay/live overlap once and preserves the next contiguous event', async () => {
    let deliver!: (event: EventEnvelope) => void;
    transport.listen.mockImplementation(async (listener) => {
      deliver = listener;
      return vi.fn();
    });
    transport.replay.mockImplementation(async () => {
      deliver(toast(1, 'Overlapping delivery'));
      deliver(toast(2, 'Buffered live delivery'));
      return [toast(1, 'Overlapping delivery')];
    });
    const window = await runtime();
    await window.app.startApp();
    deliver(toast(2, 'Duplicate live delivery'));
    deliver(toast(3, 'Next delivery'));

    expect(window.toastStore.getSnapshot().history.map((entry) => entry.message)).toEqual([
      'Next delivery', 'Buffered live delivery', 'Overlapping delivery',
    ]);
    expect(transport.bootstrap).toHaveBeenCalledTimes(1);
    expect(transport.ready).toHaveBeenCalledTimes(1);
  });

  it('rebootstraps a gap and ignores callbacks belonging to the discarded generation', async () => {
    const listeners: Array<(event: EventEnvelope) => void> = [];
    const stops: ReturnType<typeof vi.fn>[] = [];
    transport.listen.mockImplementation(async (listener) => {
      listeners.push(listener);
      const stop = vi.fn();
      stops.push(stop);
      return stop;
    });
    transport.replay.mockResolvedValueOnce([toast(1, 'Before recovery')])
      .mockResolvedValue([toast(1, 'Before recovery'), toast(2, 'Recovered event')]);
    const window = await runtime();
    await window.app.startApp();
    listeners[0](toast(3, 'Must not apply across a gap'));
    await vi.waitFor(() => expect(listeners).toHaveLength(2));
    await vi.waitFor(() => expect(window.toastStore.getSnapshot().history.some(
      (entry) => entry.message === 'Recovered event',
    )).toBe(true));
    listeners[0](toast(3, 'Obsolete listener'));
    listeners[1](toast(3, 'Current listener'));

    expect(transport.bootstrap).toHaveBeenCalledTimes(2);
    expect(stops[0]).toHaveBeenCalledTimes(1);
    expect(window.toastStore.getSnapshot().history.map((entry) => entry.message)).toContain('Current listener');
    expect(window.toastStore.getSnapshot().history.some((entry) =>
      ['Must not apply across a gap', 'Obsolete listener'].includes(entry.message),
    )).toBe(false);
  });

  it('keeps module-local selections independent while both runtimes consume shared events', async () => {
    const snapshot = makeBootstrap({
      chat: {
        sessions: [
          { session_id: 'first', title: 'First', message_ids: ['first-reply'], active: true },
          { session_id: 'second', title: 'Second', message_ids: ['second-reply'], active: false },
        ],
        messages: [
          { message_id: 'first-reply', session_id: 'first', role: 'assistant', text: 'First transcript' },
          { message_id: 'second-reply', session_id: 'second', role: 'assistant', text: 'Second transcript' },
        ],
        active_session_id: 'first', active_request_id: null, active_turn_id: null, pending_approval: null,
      },
    });
    transport.bootstrap.mockResolvedValue(snapshot);
    const listeners: Array<(event: EventEnvelope) => void> = [];
    transport.listen.mockImplementation(async (listener) => {
      listeners.push(listener);
      return vi.fn();
    });
    const main = await runtime();
    await main.app.startApp();
    const agents = await runtime();
    await agents.app.startApp();
    expect(await agents.agentStore.requestSwitchSession('second')).toBe(true);
    const event = makeEvent(1, { type: 'todos_changed', payload: {
      session_id: 'first', todos: [{ id: 'task', title: 'First session task', status: 'in_progress' }],
    } });
    listeners.forEach((deliver) => deliver(event));
    listeners.forEach((deliver) => deliver(toast(2, 'Global event after background session event')));

    expect(main.agentStore).not.toBe(agents.agentStore);
    expect(main.agentStore.getSnapshot().activeSessionId).toBe('first');
    expect(agents.agentStore.getSnapshot().activeSessionId).toBe('second');
    expect(main.agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['First transcript']);
    expect(agents.agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['Second transcript']);
    expect(main.agentStore.getSnapshot().todos[0]?.text).toBe('First session task');
    expect(agents.agentStore.getSnapshot().todos).toEqual([]);
    expect(transport.bootstrap).toHaveBeenCalledTimes(2);
    expect(transport.switchSession).not.toHaveBeenCalled();
    for (const window of [main, agents]) {
      expect(window.toastStore.getSnapshot().history[0]?.message).toBe('Global event after background session event');
    }
    agents.agentStore.selectSessionLocally('first');
    expect(agents.agentStore.getSnapshot().todos[0]?.text).toBe('First session task');
  });
});
