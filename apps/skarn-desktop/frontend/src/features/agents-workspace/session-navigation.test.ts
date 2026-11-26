import { describe, expect, it } from 'vitest';
import type { AgentSession, ChatMessage, ResumableAgentSession } from '../agent/types';
import { filterNavigationSessions, groupNavigationSessions, mergeNavigationSessions, sessionAttention } from './session-navigation';

const live = (id: string, extra: Partial<AgentSession> = {}): AgentSession => ({
  id, title: id, createdAt: Date.now(), updatedAt: Date.now(), tokenCount: 0, costUsd: 0, ...extra,
});
const durable = (id: string, extra: Partial<ResumableAgentSession> = {}): ResumableAgentSession => ({
  id, title: id, updatedAt: 1000, mode: 'full', compacted: false, messageCount: 1, ...extra,
});

describe('session navigation projection', () => {
  it('deduplicates live and durable IDs while preserving running status and durable metadata', () => {
    const rows = mergeNavigationSessions([live('same', { title: 'Live title', isRunning: true })],
      [durable('same', { model: 'model-a', pending: { kind: 'approval', summary: 'Review' } }), durable('closed')], ['closed']);
    expect(rows.map((row) => row.id)).toEqual(['closed', 'same']);
    expect(rows[1]).toMatchObject({ title: 'Live title', updatedAt: 1000, isLive: true,
      isRunning: true, model: 'model-a', pending: { kind: 'approval' } });
  });

  it('never uses fabricated live timestamps or invalid durable dates', () => {
    const rows = mergeNavigationSessions([live('live')], [durable('bad', { updatedAt: NaN })], []);
    expect(rows.every((row) => row.updatedAt === undefined)).toBe(true);
    expect(groupNavigationSessions(rows).map((group) => group.label)).toEqual(['Undated']);
  });

  it('trusts snapshot-marked live dates and retains durable fallback', () => {
    const rows = mergeNavigationSessions([
      live('snapshot', { durable: false, updatedAt: 2000 }),
      live('model-marker', { modelId: null, updatedAt: 3000 }),
      live('fallback', { durable: true, updatedAt: NaN, createdAt: NaN }),
      live('created', { durable: true, updatedAt: NaN, createdAt: 4000 }),
      live('legacy'),
      live('unknown', { durable: false, updatedAt: null, createdAt: null }),
    ], [durable('fallback')], []);
    expect(Object.fromEntries(rows.map((row) => [row.id, row.updatedAt]))).toEqual({
      snapshot: 2000, 'model-marker': 3000, fallback: 1000, created: 4000, legacy: undefined, unknown: undefined,
    });
  });

  it('includes pending durable sessions and supplied transcript attention IDs', () => {
    const rows = mergeNavigationSessions([live('failed')], [
      durable('pending', { pending: { kind: 'approval', summary: 'Waiting' } }), durable('quiet'),
    ], [], ['failed']);
    expect(filterNavigationSessions(rows, '', 'attention').map((row) => row.id).sort())
      .toEqual(['failed', 'pending']);
  });

  it('derives errors and approval attention from real transcript states', () => {
    const message = (extra: Partial<ChatMessage>): ChatMessage => ({
      id: 'message', role: 'assistant', content: '', status: 'complete', timestamp: 0, ...extra,
    });
    const tool = { id: 'tool', toolName: 'bash', argsSummary: '', startedAt: 0 };
    const attention = sessionAttention({
      messageError: [message({ status: 'error' })],
      toolError: [message({ toolCalls: [{ ...tool, status: 'error' }] })],
      approval: [message({ toolCalls: [{ ...tool, status: 'needs_approval' }] })],
      quiet: [message({})],
    });
    expect([...attention.entries()]).toEqual([
      ['messageError', 'error'], ['toolError', 'error'], ['approval', 'approval'],
    ]);
  });

  it('searches title, ID and model with running and pin filters', () => {
    const rows = mergeNavigationSessions([live('one', { title: 'Build App', isRunning: true })],
      [durable('two', { model: 'special-model' })], ['two']);
    expect(filterNavigationSessions(rows, ' BUILD ', 'running').map((row) => row.id)).toEqual(['one']);
    expect(filterNavigationSessions(rows, 'special', 'pinned').map((row) => row.id)).toEqual(['two']);
    expect(filterNavigationSessions(rows, 'two', 'running')).toEqual([]);
  });

  it('groups only real durable dates with local calendar boundaries', () => {
    const now = new Date(2026, 9, 30, 12);
    const rows = mergeNavigationSessions([live('undated')], [
      durable('today', { updatedAt: new Date(2026, 9, 30, 8).getTime() }),
      durable('yesterday', { updatedAt: new Date(2026, 9, 29, 23).getTime() }),
      durable('earlier'), durable('pin'),
    ], ['pin']);
    expect(groupNavigationSessions(rows, now).map((group) => group.label))
      .toEqual(['Pinned', 'Today', 'Yesterday', 'Earlier', 'Undated']);
  });
});
