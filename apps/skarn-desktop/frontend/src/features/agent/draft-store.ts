export type AgentDraft = {
  text: string;
  attachments: string[];
  selectionStart?: number;
  selectionEnd?: number;
  revision: number;
};

type Surface = 'main' | 'agents';
const drafts = new Map<string, AgentDraft>();
const listeners = new Set<() => void>();
const emptyDraft: AgentDraft = { text: '', attachments: [], revision: 0 };

function key(workspaceId: string, sessionId: string, surface: Surface): string {
  return `skarn:agent-draft:${JSON.stringify([surface, workspaceId, sessionId])}`;
}

export function readAgentDraft(workspaceId: string, sessionId: string, surface: Surface): AgentDraft {
  const id = key(workspaceId, sessionId, surface);
  const cached = drafts.get(id);
  if (cached) return cached;
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(id) ?? 'null');
    if (raw && typeof raw === 'object') {
      const value = raw as AgentDraft;
      if (typeof value.text === 'string' && Array.isArray(value.attachments) &&
          value.attachments.every((path) => typeof path === 'string') &&
          Number.isSafeInteger(value.revision) && value.revision >= 0) {
        drafts.set(id, value);
        return value;
      }
    }
  } catch { /* Storage is optional in restricted previews. */ }
  drafts.set(id, emptyDraft);
  return emptyDraft;
}

export function writeAgentDraft(
  workspaceId: string, sessionId: string, surface: Surface, value: AgentDraft,
): void {
  const current = readAgentDraft(workspaceId, sessionId, surface);
  const next = { ...value, attachments: [...value.attachments], revision: current.revision + 1 };
  const id = key(workspaceId, sessionId, surface);
  drafts.set(id, next);
  try { localStorage.setItem(id, JSON.stringify(next)); } catch { /* Keep the in-memory draft. */ }
  for (const listener of listeners) listener();
}

/** Compare and clear synchronously, so an accepted send cannot erase newer typing. */
export function clearAgentDraft(
  workspaceId: string, sessionId: string, surface: Surface, expectedRevision?: number,
): boolean {
  const current = readAgentDraft(workspaceId, sessionId, surface);
  if (expectedRevision !== undefined && current.revision !== expectedRevision) return false;
  writeAgentDraft(workspaceId, sessionId, surface, { ...emptyDraft, revision: current.revision });
  return true;
}

export const draftStore = {
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  },
  read: readAgentDraft,
  write: writeAgentDraft,
  clear: clearAgentDraft,
};
