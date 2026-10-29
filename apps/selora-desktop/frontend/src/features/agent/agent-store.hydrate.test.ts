// Regression: a bootstrap that arrives after SendAgent must not wipe the live
// transcript. The user-visible symptom was a prompt that only appeared after
// switching sessions (which replayed the client-side session view).
import { describe, expect, it } from 'vitest';

import type { ChatMessage } from './types';
import { mergeHydratedTranscript } from './agent-store';

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
});