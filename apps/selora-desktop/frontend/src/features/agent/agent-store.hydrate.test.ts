// Regression: a bootstrap that arrives after SendAgent must not wipe the live
// transcript. The user-visible symptom was a prompt that only appeared after
// switching sessions (which replayed the client-side session view).
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { BootstrapState, ChatMessage as WireMessage } from '../../bridge/bridge';
import { retryMessage, sendAgent, stopChat } from '../../bridge/bridge';
import { makeBootstrap } from '../../testing/fixtures';
import type { ChatMessage } from './types';
import { agentStore, mergeHydratedTranscript } from './agent-store';

vi.mock('../../bridge/bridge', async (importOriginal) => ({
  ...await importOriginal<typeof import('../../bridge/bridge')>(),
  sendAgent: vi.fn(),
  stopChat: vi.fn().mockResolvedValue(undefined),
  retryMessage: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../sound/sound-store', () => ({ playCue: vi.fn() }));

const PARENT = 'session-parent';
const OTHER = 'session-other';
const REQUEST = 'request-parent';

function wire(id: string, role: 'user' | 'assistant', text: string, sessionId = PARENT): WireMessage {
  return { message_id: id, session_id: sessionId, role, text };
}

function bootstrap(chat: Partial<BootstrapState['chat']> = {}): BootstrapState {
  return makeBootstrap({
    workspace: { id: 'ws', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
    models: [{ id: 'model', display_name: 'Model', available: true }],
    active_model_id: 'model',
    chat: {
      sessions: [
        { session_id: PARENT, title: 'Parent', message_ids: [], active: false },
        { session_id: OTHER, title: 'Other', message_ids: [], active: false },
      ],
      messages: [],
      active_session_id: PARENT,
      active_request_id: null,
      active_turn_id: null,
      pending_approval: null,
      ...chat,
    },
  });
}

function liveBootstrap(messages: WireMessage[] = []): BootstrapState {
  const snapshot = bootstrap({ messages, active_request_id: REQUEST });
  snapshot.chat.sessions[0]!.active = true;
  return snapshot;
}

beforeEach(() => {
  agentStore.reset();
  vi.mocked(sendAgent).mockReset();
  vi.mocked(sendAgent).mockReturnValue({ requestId: REQUEST, promise: new Promise(() => {}) });
  vi.mocked(stopChat).mockClear();
  vi.mocked(retryMessage).mockClear();
});

function user(content: string): ChatMessage {
  return { id: `u-${content}`, role: 'user', content, status: 'complete', timestamp: 1 };
}

function assistant(content: string): ChatMessage {
  return {
    id: `a-${content}`,
    role: 'assistant',
    content,
    status: 'streaming',
    timestamp: 1,
  };
}

describe('bootstrap transcript hydration', () => {
  it('keeps the live transcript when a mid-turn snapshot is behind', () => {
    const local = [user('run the tests'), assistant('Running')];
    const remote = [user('run the tests')];

    expect(mergeHydratedTranscript(local, remote, true)).toBe(local);
  });

  it('adopts the snapshot once the turn is no longer live', () => {
    const local = [user('run the tests'), assistant('Running')];
    const remote = [user('run the tests'), assistant('All green')];

    expect(mergeHydratedTranscript(local, remote, false)).toEqual(remote);
  });

  it('never drops a longer live history in favour of an empty snapshot', () => {
    const local = [user('hello')];

    expect(mergeHydratedTranscript(local, [], true)).toEqual(local);
  });

  it('uses the snapshot on a fresh boot with no local messages', () => {
    const remote = [user('restored')];

    expect(mergeHydratedTranscript([], remote, false)).toEqual(remote);
    expect(mergeHydratedTranscript([], remote, true)).toEqual(remote);
  });

  it('keeps newer streamed text when a live snapshot has the same row count', () => {
    const local = [user('hello'), assistant('Newer streamed text')];
    const remote = [user('hello'), assistant('Newer')];

    expect(mergeHydratedTranscript(local, remote, true)).toEqual(local);
  });
});

describe('store hydrate and event regressions', () => {
  it('keeps one optimistic prompt and its stream across a lagging send bootstrap', () => {
    agentStore.hydrate(bootstrap());
    expect(agentStore.sendUserMessage('run the tests')).toBe(true);
    const updates: string[][] = [];
    const unsubscribe = agentStore.subscribe(() => {
      updates.push(agentStore.getSnapshot().messages.map((message) => message.content));
    });

    agentStore.hydrate(liveBootstrap([wire('backend-user', 'user', 'run the tests')]));
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'Running' });
    unsubscribe();

    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['run the tests', 'Running']);
    expect(updates.every((contents) => contents[0] === 'run the tests')).toBe(true);
  });

  it('keeps same-length lagging snapshot text behind the local stream', () => {
    agentStore.hydrate(liveBootstrap([
      wire('u', 'user', 'hello'), wire('a', 'assistant', 'First'),
    ]));
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: ' second' });
    agentStore.hydrate(liveBootstrap([
      wire('u', 'user', 'hello'), wire('a', 'assistant', 'First'),
    ]));
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: ' third' });

    expect(agentStore.getSnapshot().messages[1]!.content).toBe('First second third');
  });

  it('opens an assistant row when a fresh live bootstrap contains only the persisted user', () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'First' });
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: ' second' });

    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['hello', 'First second']);
    expect(agentStore.getSnapshot().messages[1]!.status).toBe('streaming');
  });

  it('accepts completion after reconnect even if no delta created an assistant row', () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleChatCompleted({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', message_id: 'a',
      text: 'Final answer', total_tokens: 1, cost_usd: 0,
    });

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({
      id: 'a', role: 'assistant', content: 'Final answer', status: 'complete',
    });
  });

  it('takes same-id terminal corrections and authoritative history truncation', () => {
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Old answer')] }));
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Corrected answer')] }));
    expect(agentStore.getSnapshot().messages[1]!.content).toBe('Corrected answer');
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));
    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['hello']);
  });

  it('adopts authoritative terminal snapshot text without duplicating optimistic ids', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'partial' });
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: true,
      end_message: 'done', total_tokens: 12, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Final answer')] }));

    expect(agentStore.getSnapshot().messages.map(({ id, content, status }) => ({ id, content, status }))).toEqual([
      { id: 'u', content: 'hello', status: 'complete' },
      { id: 'a', content: 'Final answer', status: 'complete' },
    ]);
    expect(agentStore.getSnapshot().inFlight).toBe(false);
  });

  it('adopts a persisted current-turn completion when the terminal event was missed', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'partial' });
    const previous = agentStore.getSnapshot();
    const original = structuredClone(previous);
    Object.freeze(previous.messages);

    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Final answer')] }));

    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().thinkingState).toBe('breathing');
    expect(agentStore.getSnapshot().streamingActivityLevel).toBe(0);
    expect(agentStore.getSnapshot().messages.map(({ id, content, status }) => ({ id, content, status }))).toEqual([
      { id: 'u', content: 'hello', status: 'complete' },
      { id: 'a', content: 'Final answer', status: 'complete' },
    ]);
    expect(previous).toEqual(original);
    agentStore.selectSessionLocally(OTHER);
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().thinkingState).toBe('breathing');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'late' });
    expect(agentStore.getSnapshot().messages[1]!.content).toBe('Final answer');
  });

  it('does not finish a new turn from a stale prior transcript, even when the prompt repeats', () => {
    const history = [wire('old-u', 'user', 'hello'), wire('old-a', 'assistant', 'Old answer')];
    agentStore.hydrate(bootstrap({ messages: history }));
    agentStore.sendUserMessage('hello');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'New partial' });

    agentStore.hydrate(bootstrap({ messages: history }));
    expect(agentStore.getSnapshot().inFlight).toBe(true);
    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual([
      'hello', 'Old answer', 'hello', 'New partial',
    ]);
    agentStore.hydrate(bootstrap({ messages: [...history, wire('new-u', 'user', 'hello')] }));
    expect(agentStore.getSnapshot().inFlight).toBe(true);
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: ' continued' });
    expect(agentStore.getSnapshot().messages[3]!.content).toBe('New partial continued');

    agentStore.hydrate(bootstrap({ messages: [
      ...history, wire('new-u', 'user', 'hello'), wire('new-a', 'assistant', 'New final answer'),
    ] }));
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().messages[3]).toMatchObject({ id: 'new-a', content: 'New final answer', status: 'complete' });
  });

  it('does not treat an inactive different prompt at the same position as current-turn completion', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('Current prompt');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'Current partial' });
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'Different prompt'), wire('a', 'assistant', 'Different answer')] }));

    expect(agentStore.getSnapshot().inFlight).toBe(true);
    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['Current prompt', 'Current partial']);
  });

  it('keeps the local turn live when the snapshot still reports a backend request', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'Newer partial' });
    const snapshot = bootstrap({
      active_request_id: REQUEST,
      messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Older partial')],
    });
    agentStore.hydrate(snapshot);

    expect(agentStore.getSnapshot().inFlight).toBe(true);
    expect(agentStore.getSnapshot().messages[1]!.content).toBe('Newer partial');
  });

  it('uses authoritative chat_completed text and id, even when text is empty', () => {
    for (const text of ['Complete answer', '']) {
      agentStore.reset();
      agentStore.hydrate(bootstrap());
      agentStore.sendUserMessage('hello');
      agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'partial' });
      agentStore.handleChatCompleted({
        request_id: REQUEST, session_id: PARENT, turn_id: 'turn', message_id: 'backend-assistant',
        text, total_tokens: 17, cost_usd: 0.1,
      });
      expect(agentStore.getSnapshot().messages.at(-1)).toMatchObject({
        id: 'backend-assistant', content: text, status: 'complete',
      });
      expect(agentStore.getSnapshot().metrics.totalTokens).toBe(17);
      expect(agentStore.getSnapshot().inFlight).toBe(false);
    }
  });

  it('keeps tool cards on the authoritative terminal row, not a duplicate optimistic row', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'partial' });
    agentStore.handleToolCallStarted({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn',
      call_id: 'call', tool_name: 'read_file', args_json: '{"path":"README.md"}',
    });
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: true,
      end_message: 'done', total_tokens: 12, cost_usd: 0, num_tools: 1, approved: 0, denied: 0,
    });
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Final answer')] }));

    expect(agentStore.getSnapshot().messages).toHaveLength(2);
    expect(agentStore.getSnapshot().messages[1]).toMatchObject({
      id: 'a', content: 'Final answer', toolCalls: [{ id: 'call' }],
    });
  });

  it('never mutates a previously published transcript when hydrating new rows or approvals', () => {
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));
    const previous = agentStore.getSnapshot();
    const original = structuredClone(previous);
    Object.freeze(previous.messages);

    agentStore.hydrate(bootstrap({
      messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Answer')],
      pending_approval: {
        approval_id: 'approval', request_id: REQUEST, turn_id: 'turn', session_id: PARENT,
        summary: 'read file', can_approve_all_this_turn: true,
      },
    }));

    expect(previous).toEqual(original);
    expect(agentStore.getSnapshot().messages[1]!.toolCalls).toHaveLength(1);
  });

  it('keeps live approval hydration immutable too', () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello'), wire('a', 'assistant', 'Partial')]));
    const previous = agentStore.getSnapshot();
    const original = structuredClone(previous);
    Object.freeze(previous.messages);

    const snapshot = liveBootstrap([wire('u', 'user', 'hello')]);
    snapshot.chat.pending_approval = {
      approval_id: 'approval', request_id: REQUEST, turn_id: 'turn', session_id: PARENT,
      summary: 'read file', can_approve_all_this_turn: true,
    };
    agentStore.hydrate(snapshot);

    expect(previous).toEqual(original);
    expect(agentStore.getSnapshot().messages[1]!.toolCalls).toHaveLength(1);
  });

  it('keeps bootstrap during send from clearing routing, flight state, or stop target', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.hydrate(bootstrap());
    expect(agentStore.getSnapshot().inFlight).toBe(true);
    agentStore.handleChatDelta({ request_id: 'wrong-request', session_id: PARENT, text: 'wrong' });
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'right' });
    expect(agentStore.getSnapshot().messages.at(-1)!.content).toBe('right');

    agentStore.stopActiveTurn();
    expect(stopChat).toHaveBeenCalledWith(REQUEST, expect.objectContaining({ session_id: PARENT }));
  });

  it('persists the send request before switching sessions without another event', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.selectSessionLocally(OTHER);
    agentStore.selectSessionLocally(PARENT);
    agentStore.handleChatDelta({ request_id: 'wrong-request', session_id: PARENT, text: 'wrong' });
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'right' });

    expect(agentStore.getSnapshot().messages.at(-1)!.content).toBe('right');
  });

  it('routes background stream and hydration without leaking into the selected session', () => {
    agentStore.hydrate(bootstrap({ messages: [wire('other-user', 'user', 'Other prompt', OTHER)] }));
    agentStore.sendUserMessage('Parent prompt');
    agentStore.selectSessionLocally(OTHER);
    const apply = () => agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'Parent stream' });
    expect(agentStore.routeSessionEvent({ request_id: REQUEST, session_id: PARENT }, apply)).toBe(true);
    agentStore.hydrate(bootstrap({
      active_session_id: OTHER,
      messages: [wire('other-user', 'user', 'Other prompt', OTHER), wire('backend-user', 'user', 'Parent prompt')],
    }));

    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['Other prompt']);
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['Parent prompt', 'Parent stream']);
    expect(agentStore.getSnapshot().inFlight).toBe(true);
  });

  it('applies a background terminal event only to its source session', () => {
    agentStore.hydrate(bootstrap({ messages: [wire('other-user', 'user', 'Other prompt', OTHER)] }));
    agentStore.sendUserMessage('Parent prompt');
    agentStore.selectSessionLocally(OTHER);
    const payload = {
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', message_id: 'backend-assistant',
      text: 'Parent answer', total_tokens: 19, cost_usd: 0,
    };
    agentStore.routeSessionEvent(payload, () => agentStore.handleChatCompleted(payload));

    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['Other prompt']);
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().messages[1]).toMatchObject({
      id: 'backend-assistant', content: 'Parent answer', status: 'complete',
    });
    expect(agentStore.getSnapshot().metrics.totalTokens).toBe(19);
    expect(agentStore.getSnapshot().inFlight).toBe(false);
  });

  it('adopts a different backend request instead of routing it to the previous turn', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('Old prompt');
    const next = liveBootstrap([wire('new-user', 'user', 'New prompt')]);
    next.chat.active_request_id = 'new-request';
    agentStore.hydrate(next);
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'old' });
    agentStore.handleChatDelta({ request_id: 'new-request', session_id: PARENT, text: 'new' });

    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['New prompt', 'new']);
  });

  it('keeps a rejected send scoped to its source session after switching tabs', async () => {
    let rejectSend!: (error: unknown) => void;
    vi.mocked(sendAgent).mockReturnValue({
      requestId: REQUEST,
      promise: new Promise((_, reject) => { rejectSend = reject; }),
    });
    agentStore.hydrate(bootstrap({ messages: [
      wire('other-u', 'user', 'Other prompt', OTHER), wire('other-a', 'assistant', 'Other answer', OTHER),
    ] }));
    agentStore.sendUserMessage('Parent prompt');
    agentStore.selectSessionLocally(OTHER);
    rejectSend({ message: 'transport failed' });
    await Promise.resolve();

    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['Other prompt', 'Other answer']);
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ status: 'error' });
    expect(agentStore.getSnapshot().messages[1]!.content).toContain('transport failed');
    expect(agentStore.getSnapshot().inFlight).toBe(false);
  });

  it('ignores a send rejection after completion and workspace replacement', async () => {
    let rejectSend!: (error: unknown) => void;
    vi.mocked(sendAgent).mockReturnValue({
      requestId: REQUEST,
      promise: new Promise((_, reject) => { rejectSend = reject; }),
    });
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatCompleted({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', message_id: 'a',
      text: 'Done', total_tokens: 1, cost_usd: 0,
    });
    rejectSend({ message: 'late transport failure' });
    await Promise.resolve();
    expect(agentStore.getSnapshot().messages[1]!.content).toBe('Done');

    agentStore.reset();
    vi.mocked(sendAgent).mockReturnValue({
      requestId: 'request-new',
      promise: new Promise((_, reject) => { rejectSend = reject; }),
    });
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('old workspace');
    const replacement = bootstrap({ messages: [wire('u', 'user', 'New workspace'), wire('a', 'assistant', 'New answer')] });
    replacement.workspace.id = 'replacement-workspace';
    agentStore.hydrate(replacement);
    rejectSend({ message: 'old transport failure' });
    await Promise.resolve();
    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['New workspace', 'New answer']);
  });

  it('does not resurrect a completed request from a lagging live bootstrap', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatCompleted({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', message_id: 'a',
      text: 'Done', total_tokens: 1, cost_usd: 0,
    });
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello'), wire('a', 'assistant', 'Done')]));
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'late' });

    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().messages.at(-1)!.content).toBe('Done');
  });

  it('keeps the final transcript when a terminal request reappears with only stale user history', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatCompleted({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', message_id: 'a',
      text: 'Final answer', total_tokens: 1, cost_usd: 0,
    });
    const completed = agentStore.getSnapshot().messages;
    const stale = liveBootstrap([wire('u', 'user', 'hello')]);
    stale.chat.pending_approval = {
      approval_id: 'stale-approval', request_id: REQUEST, session_id: PARENT, turn_id: 'turn',
      summary: 'Old approval', can_approve_all_this_turn: true,
    };
    agentStore.hydrate(stale);

    expect(agentStore.getSnapshot().messages).toEqual(completed);
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().sessions.find((session) => session.id === PARENT)!.isRunning).toBe(false);
    agentStore.selectSessionLocally(OTHER);
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().messages).toEqual(completed);
    expect(agentStore.getSnapshot().inFlight).toBe(false);
  });

  it('keeps an omitted failed assistant row when the idle snapshot still contains its prompt', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Provider refused the request', total_tokens: 0, cost_usd: 0,
      num_tools: 0, approved: 0, denied: 0,
    });
    const previous = agentStore.getSnapshot();
    const original = structuredClone(previous);
    Object.freeze(previous.messages);
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));

    expect(agentStore.getSnapshot().messages).toHaveLength(2);
    expect(agentStore.getSnapshot().messages[1]).toMatchObject({
      status: 'error', content: 'Provider refused the request',
    });
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(previous).toEqual(original);
  });

  it('keeps chat_failed status and its surfaced error while adopting persisted assistant text', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'partial' });
    agentStore.handleChatFailed({ request_id: REQUEST, session_id: PARENT, error: { message: 'Gateway timeout' } });
    expect(agentStore.getSnapshot().messages[1]!.status).toBe('error');

    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Persisted partial')] }));
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Persisted partial')] }));

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({
      id: 'a', status: 'error', content: 'Persisted partial\n\n[Error: Gateway timeout]',
    });
  });

  it('keeps a surfaced send rejection across idle hydration with no persisted assistant', async () => {
    vi.mocked(sendAgent).mockReturnValue({ requestId: REQUEST, promise: Promise.reject({ message: 'Transport failed' }) });
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    await Promise.resolve();
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({
      status: 'error', content: 'Could not reach the assistant: Transport failed',
    });
  });

  it('keeps cancellation metadata on the matching persisted row without replacing persisted text', () => {
    agentStore.hydrate(bootstrap());
    agentStore.sendUserMessage('hello');
    agentStore.handleChatDelta({ request_id: REQUEST, session_id: PARENT, text: 'partial' });
    agentStore.stopActiveTurn();
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('a', 'assistant', 'Persisted partial')] }));

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({
      id: 'a', status: 'complete', content: 'Persisted partial', cancelled: true,
    });
  });

  it('drops a local failure when authoritative history removes its prompt', () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Provider refused the request', total_tokens: 0, cost_usd: 0,
      num_tools: 0, approved: 0, denied: 0,
    });
    agentStore.hydrate(bootstrap());

    expect(agentStore.getSnapshot().messages).toEqual([]);
    agentStore.selectSessionLocally(OTHER);
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().messages).toEqual([]);
  });

  it('does not restore a failure after an explicit backend retry truncates the failed tail', async () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Provider refused the request', total_tokens: 0, cost_usd: 0,
      num_tools: 0, approved: 0, denied: 0,
    });
    expect(await agentStore.retryMessage('u')).toBe(true);
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));

    expect(agentStore.getSnapshot().messages.map((message) => message.content)).toEqual(['hello']);
    expect(retryMessage).toHaveBeenCalledWith(PARENT, 'u', expect.objectContaining({ session_id: PARENT }));
  });

  it('does not apply failure metadata to a replacement turn with the same prompt text', () => {
    agentStore.hydrate(liveBootstrap([wire('old-u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Provider refused the request', total_tokens: 0, cost_usd: 0,
      num_tools: 0, approved: 0, denied: 0,
    });
    agentStore.hydrate(bootstrap({ messages: [wire('new-u', 'user', 'hello'), wire('new-a', 'assistant', 'New answer')] }));

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ id: 'new-a', content: 'New answer', status: 'complete' });
  });

  it('does not apply an old failure to a live backend retry of the same persisted prompt', () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Old failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    const replacement = liveBootstrap([wire('u', 'user', 'hello')]);
    replacement.chat.active_request_id = 'retry-request';
    agentStore.hydrate(replacement);
    agentStore.handleChatDelta({ request_id: 'retry-request', session_id: PARENT, text: 'New answer' });

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ status: 'streaming', content: 'New answer' });
    expect(agentStore.getSnapshot().messages[1]).not.toHaveProperty('terminalError');
  });

  it('retains failure metadata when the explicit retry command itself fails', async () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Original failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    vi.mocked(retryMessage).mockRejectedValueOnce({ message: 'Retry refused' });
    expect(await agentStore.retryMessage('u')).toBe(false);
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ status: 'error', content: 'Original failure' });
  });

  it('restores the unchanged failed tail when hydration hid it before retry rejection', async () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Original failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    const failed = agentStore.getSnapshot().messages[1];
    let rejectRetry!: (error: unknown) => void;
    vi.mocked(retryMessage).mockReturnValueOnce(new Promise((_, reject) => { rejectRetry = reject; }));
    const pending = agentStore.retryMessage('u');
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));
    expect(agentStore.getSnapshot().messages).toHaveLength(1);

    rejectRetry({ message: 'Retry refused' });
    expect(await pending).toBe(false);
    expect(agentStore.getSnapshot().messages[1]).toEqual(failed);
    expect(agentStore.getSnapshot().inFlight).toBe(false);
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));
    expect(agentStore.getSnapshot().messages[1]).toEqual(failed);
  });

  it('restores a rejected retry only in its parked source session, not the selected session', async () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Parent failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    let rejectRetry!: (error: unknown) => void;
    vi.mocked(retryMessage).mockReturnValueOnce(new Promise((_, reject) => { rejectRetry = reject; }));
    const pending = agentStore.retryMessage('u');
    agentStore.hydrate(bootstrap({ active_session_id: OTHER, messages: [
      wire('u', 'user', 'hello'), wire('other-u', 'user', 'Other prompt', OTHER),
      wire('other-a', 'assistant', 'Other answer', OTHER),
    ] }));
    const selected = agentStore.getSnapshot().messages;

    rejectRetry({ message: 'Retry refused' });
    expect(await pending).toBe(false);
    expect(agentStore.getSnapshot().activeSessionId).toBe(OTHER);
    expect(agentStore.getSnapshot().messages).toBe(selected);
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ status: 'error', content: 'Parent failure' });
  });

  it('does not restore a rejected retry over a newer turn', async () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Old failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    let rejectRetry!: (error: unknown) => void;
    vi.mocked(retryMessage).mockReturnValueOnce(new Promise((_, reject) => { rejectRetry = reject; }));
    const pending = agentStore.retryMessage('u');
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));
    vi.mocked(sendAgent).mockReturnValue({ requestId: 'new-request', promise: new Promise(() => {}) });
    agentStore.sendUserMessage('New prompt');
    agentStore.handleChatDelta({ request_id: 'new-request', session_id: PARENT, text: 'New answer' });
    const current = agentStore.getSnapshot().messages;

    rejectRetry({ message: 'Retry refused' });
    expect(await pending).toBe(false);
    expect(agentStore.getSnapshot().messages).toBe(current);
    expect(agentStore.getSnapshot().inFlight).toBe(true);
  });

  it.each(['event', 'snapshot'])('does not restore a rejected retry after a newer turn completes via %s and returns to user-only history', async (completion) => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Old failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    let rejectRetry!: (error: unknown) => void;
    vi.mocked(retryMessage).mockReturnValueOnce(new Promise((_, reject) => { rejectRetry = reject; }));
    const pending = agentStore.retryMessage('u');
    const replacement = liveBootstrap([wire('u', 'user', 'hello')]);
    replacement.chat.active_request_id = 'new-request';
    agentStore.hydrate(replacement);
    if (completion === 'event') {
      agentStore.handleChatCompleted({
        request_id: 'new-request', session_id: PARENT, turn_id: 'new-turn', message_id: 'new-a',
        text: 'New final answer', total_tokens: 3, cost_usd: 0,
      });
    } else {
      agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello'), wire('new-a', 'assistant', 'New final answer')] }));
    }
    agentStore.hydrate(bootstrap({ messages: [wire('u', 'user', 'hello')] }));
    const current = agentStore.getSnapshot().messages;

    rejectRetry({ message: 'Retry refused' });
    expect(await pending).toBe(false);
    expect(agentStore.getSnapshot().messages).toBe(current);
    expect(agentStore.getSnapshot().messages).toHaveLength(1);
  });

  it('does not restore a rejected retry after workspace replacement, even with the same prompt id', async () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Old workspace failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    let rejectRetry!: (error: unknown) => void;
    vi.mocked(retryMessage).mockReturnValueOnce(new Promise((_, reject) => { rejectRetry = reject; }));
    const pending = agentStore.retryMessage('u');
    const replacement = bootstrap({ messages: [wire('u', 'user', 'hello')] });
    replacement.workspace.id = 'replacement-workspace';
    agentStore.hydrate(replacement);
    const current = agentStore.getSnapshot().messages;

    rejectRetry({ message: 'Retry refused' });
    expect(await pending).toBe(false);
    expect(agentStore.getSnapshot().messages).toBe(current);
    expect(agentStore.getSnapshot().messages).toHaveLength(1);
  });

  it('matches a later turn after an omitted local failure without leaking the failure onto its answer', () => {
    agentStore.hydrate(liveBootstrap([wire('old-u', 'user', 'Old prompt')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Old failure', total_tokens: 0, cost_usd: 0, num_tools: 0, approved: 0, denied: 0,
    });
    agentStore.hydrate(bootstrap({ messages: [wire('old-u', 'user', 'Old prompt')] }));
    vi.mocked(sendAgent).mockReturnValue({ requestId: 'new-request', promise: new Promise(() => {}) });
    agentStore.sendUserMessage('New prompt');
    agentStore.handleChatDelta({ request_id: 'new-request', session_id: PARENT, text: 'New partial' });
    agentStore.hydrate(bootstrap({ messages: [
      wire('old-u', 'user', 'Old prompt'), wire('new-u', 'user', 'New prompt'), wire('new-a', 'assistant', 'New final'),
    ] }));

    expect(agentStore.getSnapshot().inFlight).toBe(false);
    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ status: 'error', content: 'Old failure' });
    expect(agentStore.getSnapshot().messages[3]).toMatchObject({ id: 'new-a', status: 'complete', content: 'New final' });
    expect(agentStore.getSnapshot().messages[3]).not.toHaveProperty('terminalError');
  });

  it('keeps failed-turn metadata scoped to its parent while hydrating another selected session', () => {
    agentStore.hydrate(liveBootstrap([wire('u', 'user', 'hello')]));
    agentStore.handleAgentTurnDone({
      request_id: REQUEST, session_id: PARENT, turn_id: 'turn', completed: false,
      end_message: 'Parent failure', total_tokens: 0, cost_usd: 0,
      num_tools: 0, approved: 0, denied: 0,
    });
    agentStore.hydrate(bootstrap({ active_session_id: OTHER, messages: [
      wire('u', 'user', 'hello'), wire('other-u', 'user', 'hello', OTHER),
      wire('other-a', 'assistant', 'Other answer', OTHER),
    ] }));

    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ id: 'other-a', content: 'Other answer', status: 'complete' });
    agentStore.selectSessionLocally(PARENT);
    expect(agentStore.getSnapshot().messages[1]).toMatchObject({ status: 'error', content: 'Parent failure' });
  });

  it('registers first prompt session without fabricated thought and preserves empty active assistant row until turn ends', () => {
    agentStore.hydrate(bootstrap({
      active_session_id: '',
      sessions: [],
      messages: [],
    }));
    expect(agentStore.sendUserMessage('first prompt')).toBe(true);

    const afterSend = agentStore.getSnapshot();
    expect(afterSend.activeSessionId).toBe('sess_default');
    expect(afterSend.messages).toHaveLength(2);
    expect(afterSend.messages[1]!.thought).toBeUndefined();
    expect(afterSend.messages[1]!.thoughtDurationSeconds).toBeUndefined();
    expect(agentStore.needsRequestHydration({ request_id: REQUEST, session_id: 'sess_default' })).toBe(false);

    // First call with activeSessionId === '' & matching workspace_id: 'ws'
    agentStore.hydrateAgents({
      protocol_version: 1,
      core_instance_id: 'core-0001',
      workspace_id: 'ws',
      snapshot_sequence: 1,
      selected_session_id: 'sess_default',
      sessions: [{
        session_id: 'sess_default',
        title: 'Default session',
        created_at: null,
        updated_at: null,
        model_id: 'test-model',
        mode: 'execute',
        durable: false,
        active_request_id: REQUEST,
        active_turn_id: null,
        state: 'thinking',
        messages: [{ message_id: 'msg-u-1', session_id: 'sess_default', role: 'user', text: 'first prompt' }],
        timeline: [],
        approvals: [],
        plan_phase: 'executing',
        plan: null,
        todos: [],
        queue: [],
        subagents: [],
      }],
    });

    const afterHydrate = agentStore.getSnapshot();
    expect(afterHydrate.messages.map((m) => m.role)).toEqual(['user', 'assistant']);
    expect(afterHydrate.messages[1]!.id).toMatch(/^msg_.*_a$/);

    agentStore.handleAgentTurnDone({
      request_id: REQUEST,
      session_id: 'sess_default',
      turn_id: 'turn-1',
      completed: false,
      end_message: 'Model call failed cleanly',
      total_tokens: 0,
      cost_usd: 0,
      num_tools: 0,
      approved: 0,
      denied: 0,
    });

    const afterDone = agentStore.getSnapshot();
    expect(afterDone.inFlight).toBe(false);
    expect(afterDone.messages.at(-1)).toMatchObject({
      role: 'assistant',
      status: 'error',
      content: 'Model call failed cleanly',
    });

    // A separate view hydrating mid-turn without an optimistic local row also keeps an active streaming slot.
    agentStore.reset();
    agentStore.hydrate(bootstrap({
      active_session_id: '',
      sessions: [],
      messages: [],
    }));
    agentStore.hydrateAgents({
      protocol_version: 1,
      core_instance_id: 'core-0001',
      workspace_id: 'ws',
      snapshot_sequence: 2,
      selected_session_id: 'sess_default',
      sessions: [{
        session_id: 'sess_default',
        title: 'Default session',
        created_at: null,
        updated_at: null,
        model_id: 'test-model',
        mode: 'execute',
        durable: false,
        active_request_id: REQUEST,
        active_turn_id: null,
        state: 'thinking',
        messages: [{ message_id: 'msg-u-1', session_id: 'sess_default', role: 'user', text: 'first prompt' }],
        timeline: [],
        approvals: [],
        plan_phase: 'executing',
        plan: null,
        todos: [],
        queue: [],
        subagents: [],
      }],
    });
    expect(agentStore.getSnapshot().messages.map((m) => m.role)).toEqual(['user', 'assistant']);
    expect(agentStore.getSnapshot().messages[1]!.id).toBe(`msg_${REQUEST}_a`);
  });

  it('surfaces IPC rejection from sendAgent even if optimistic assistant tail was missing', async () => {
    let rejectIpc!: (err: unknown) => void;
    vi.mocked(sendAgent).mockReturnValue({
      requestId: REQUEST,
      promise: new Promise((_resolve, reject) => {
        rejectIpc = reject;
      }),
    });
    agentStore.hydrate(bootstrap({ active_session_id: '', sessions: [], messages: [] }));
    expect(agentStore.sendUserMessage('run check')).toBe(true);

    // Simulate an intermediate hydrateAgents snapshot before the IPC promise rejects.
    agentStore.hydrateAgents({
      protocol_version: 1,
      core_instance_id: 'core-0001',
      workspace_id: 'ws',
      snapshot_sequence: 1,
      selected_session_id: 'sess_default',
      sessions: [{
        session_id: 'sess_default',
        title: 'Default session',
        created_at: null,
        updated_at: null,
        model_id: 'test-model',
        mode: 'execute',
        durable: false,
        active_request_id: REQUEST,
        active_turn_id: null,
        state: 'thinking',
        messages: [{ message_id: 'msg-u-1', session_id: 'sess_default', role: 'user', text: 'run check' }],
        timeline: [],
        approvals: [],
        plan_phase: 'executing',
        plan: null,
        todos: [],
        queue: [],
        subagents: [],
      }],
    });

    rejectIpc({ code: 'model_capability_unknown', message: 'Probe timed out' });
    await Promise.resolve();
    await Promise.resolve();

    const snap = agentStore.getSnapshot();
    expect(snap.inFlight).toBe(false);
    expect(snap.messages.at(-1)).toMatchObject({
      role: 'assistant',
      status: 'error',
    });
    expect(snap.messages.at(-1)!.content).toContain('Probe timed out');
  });
});
