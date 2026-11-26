import type { AgentSession, ChatMessage, ResumableAgentSession } from '../agent/types';

export type NavigationSession = {
  id: string;
  title: string;
  isRunning: boolean;
  isLive: boolean;
  pinned: boolean;
  needsAttention: boolean;
  updatedAt?: number;
  model?: string;
  branchName?: string;
  messageCount?: number;
  subagentCount?: number;
  pending?: ResumableAgentSession['pending'];
};

export type SessionFilter = 'all' | 'running' | 'pinned' | 'attention';

export function sessionAttention(transcripts: Record<string, ChatMessage[]>) {
  const attention = new Map<string, 'error' | 'approval'>();
  for (const [id, messages] of Object.entries(transcripts)) {
    if (messages.some((message) => message.status === 'error' ||
      message.toolCalls?.some((tool) => tool.status === 'error'))) attention.set(id, 'error');
    else if (messages.some((message) => message.toolCalls?.some((tool) =>
      tool.status === 'needs_approval' || String(tool.status) === 'awaiting_approval'))) attention.set(id, 'approval');
  }
  return attention;
}

function validDate(value: number | null | undefined): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 &&
    !Number.isNaN(new Date(value).getTime()) ? value : undefined;
}

export function formatSessionDate(timestamp: number) {
  const date = new Date(timestamp);
  return {
    dateTime: date.toISOString(),
    label: date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }),
    tooltip: date.toLocaleString(undefined, { dateStyle: 'full', timeStyle: 'long' }),
  };
}

/** Legacy hydrate invents dates; snapshot metadata marks authoritative live dates. */
export function mergeNavigationSessions(
  live: AgentSession[],
  durable: ResumableAgentSession[],
  pinnedIds: string[],
  attentionIds: readonly string[] = [],
): NavigationSession[] {
  const pinned = new Set(pinnedIds);
  const attention = new Set(attentionIds);
  const rows = new Map<string, NavigationSession>();
  for (const session of durable) {
    rows.set(session.id, {
      id: session.id,
      title: session.title || session.id,
      isRunning: false,
      isLive: false,
      pinned: pinned.has(session.id),
      needsAttention: attention.has(session.id) || Boolean(session.pending),
      updatedAt: validDate(session.updatedAt),
      model: session.model,
      messageCount: session.messageCount,
      pending: session.pending,
    });
  }
  for (const session of live) {
    const previous = rows.get(session.id);
    const authoritative = session.durable !== undefined || session.modelId !== undefined;
    rows.set(session.id, {
      ...previous,
      id: session.id,
      title: session.title || rows.get(session.id)?.title || session.id,
      isRunning: Boolean(session.isRunning),
      isLive: true,
      pinned: pinned.has(session.id),
      needsAttention: attention.has(session.id) || Boolean(previous?.pending),
      updatedAt: (authoritative ? validDate(session.updatedAt) ?? validDate(session.createdAt) : undefined)
        ?? previous?.updatedAt,
      model: session.modelId ?? previous?.model,
      branchName: previous?.branchName,
      messageCount: previous?.messageCount,
    });
  }
  return [...rows.values()].sort((a, b) =>
    Number(b.pinned) - Number(a.pinned) || (b.updatedAt ?? 0) - (a.updatedAt ?? 0),
  );
}

export function filterNavigationSessions(rows: NavigationSession[], query: string, filter: SessionFilter) {
  const needle = query.trim().toLocaleLowerCase();
  return rows.filter((row) =>
    (filter !== 'running' || row.isRunning) &&
    (filter !== 'pinned' || row.pinned) &&
    (filter !== 'attention' || row.needsAttention) &&
    (!needle || [row.title, row.id, row.model ?? ''].some((value) =>
      value.toLocaleLowerCase().includes(needle))),
  );
}

export function groupNavigationSessions(rows: NavigationSession[], now = new Date()) {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).getTime();
  const groups = new Map<string, NavigationSession[]>();
  for (const row of rows) {
    const label = row.pinned ? 'Pinned' : row.updatedAt === undefined ? 'Undated'
      : row.updatedAt >= today ? 'Today' : row.updatedAt >= yesterday ? 'Yesterday' : 'Earlier';
    const group = groups.get(label) ?? [];
    group.push(row);
    groups.set(label, group);
  }
  return ['Pinned', 'Today', 'Yesterday', 'Earlier', 'Undated']
    .filter((label) => groups.has(label))
    .map((label) => ({ label, sessions: groups.get(label)! }));
}
