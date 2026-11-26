// Stability: a user Stop and a stream failure must both preserve the partial
// answer. The store's pure helpers carry that contract (the React store wires
// them onto the live messages), so these tests need no bridge or DOM.
import { describe, expect, it } from 'vitest';

import type { ChatMessage } from './types';
import {
  isUserStoppedTurn,
  markActiveMessageCancelled,
  markActiveMessageFailed,
  type CancelledChatMessage,
} from './agent-store';

function assistant(content: string, status: ChatMessage['status'] = 'streaming'): ChatMessage {
  return {
    id: 'msg_a',
    role: 'assistant',
    content,
    status,
    timestamp: 1,
  };
}

function user(content: string): ChatMessage {
  return { id: 'msg_u', role: 'user', content, status: 'complete', timestamp: 1 };
}

describe('cancel keeps the partial answer', () => {
  it('marks the live assistant row cancelled without dropping its text', () => {
    const messages = [user('fix the bug'), assistant('Here is the first half')];

    const next = markActiveMessageCancelled(messages);
    const last = next[1] as CancelledChatMessage;

    expect(last.content).toBe('Here is the first half');
    expect(last.cancelled).toBe(true);
    // A stop is a lifecycle end, never an error row.
    expect(last.status).toBe('complete');
    // The user turn is untouched.
    expect(next[0]).toEqual(messages[0]);
  });

  it('is immutable: the input array keeps the streaming row', () => {
    const messages = [user('hello'), assistant('partial')];
    markActiveMessageCancelled(messages);

    expect(messages[1].status).toBe('streaming');
    expect((messages[1] as CancelledChatMessage).cancelled).toBeUndefined();
  });

  it('never rewrites a user or empty transcript', () => {
    const onlyUser = [user('hello')];
    expect(markActiveMessageCancelled(onlyUser)).toBe(onlyUser);
    expect(markActiveMessageCancelled([])).toEqual([]);
  });
});

describe('stream errors keep the partial answer and flag the row', () => {
  it('appends the surfaced error instead of replacing the text', () => {
    const messages = [user('hello'), assistant('half an answer')];

    const next = markActiveMessageFailed(
      messages,
      'The AI gateway did not answer in time. Check your connection and try again.',
    );

    expect(next[1].content).toContain('half an answer');
    expect(next[1].content).toContain('[Error: The AI gateway did not answer in time.');
    expect(next[1].status).toBe('error');
  });

  it('an empty partial row becomes the error text alone', () => {
    const next = markActiveMessageFailed([user('hello'), assistant('')], 'Cannot reach the AI gateway.');
    expect(next[1].content).toBe('[Error: Cannot reach the AI gateway.]');
    expect(next[1].status).toBe('error');
  });
});

describe('a stopped backend turn is not an error', () => {
  it('recognizes the backend stop message', () => {
    expect(isUserStoppedTurn(false, 'agent turn stopped by user')).toBe(true);
    expect(isUserStoppedTurn(false, 'Agent turn stopped by user: time cap')).toBe(true);
  });

  it('does not treat completions or real failures as stops', () => {
    expect(isUserStoppedTurn(true, 'agent turn stopped by user')).toBe(false);
    expect(isUserStoppedTurn(false, 'agent turn failed: timeout: chat stream timed out')).toBe(
      false,
    );
    expect(isUserStoppedTurn(false, undefined)).toBe(false);
  });
});