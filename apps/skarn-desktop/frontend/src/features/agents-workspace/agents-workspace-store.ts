// UI-only Agents workspace state. Protocol/session state remains in agent-store.
export type AgentsWorkspacePage = 'conversation' | 'automations' | 'customizations';
export type AgentsContextTab = 'files' | 'changes';
export type AgentsWorkspaceVisit = { page: AgentsWorkspacePage; sessionId: string | null; childId: string | null };
export type AgentsWorkspacePanel = 'navigation' | 'context';

export const AGENTS_LAYOUT_BOUNDS = {
  navigation: { min: 200, max: 360, default: 248 },
  context: { min: 240, max: 480, default: 320 },
  conversationMin: 400,
  handle: 6,
  navigationBreakpoint: 760,
  contextBreakpoint: 1120,
} as const;

export type AgentsWorkspaceState = {
  workspaceId: string | null;
  navigationWidth: number;
  contextWidth: number;
  navigationOpen: boolean;
  contextOpen: boolean;
  lastContextTab: AgentsContextTab;
  page: AgentsWorkspacePage;
  history: AgentsWorkspaceVisit[];
  historyIndex: number;
  viewportWidth: number;
  compactPanel: AgentsWorkspacePanel | null;
};

const defaults = {
  navigationWidth: AGENTS_LAYOUT_BOUNDS.navigation.default as number,
  contextWidth: AGENTS_LAYOUT_BOUNDS.context.default as number,
  navigationOpen: true,
  contextOpen: true,
  lastContextTab: 'files' as AgentsContextTab,
};
let state: AgentsWorkspaceState = {
  ...defaults, workspaceId: null, page: 'conversation', history: [], historyIndex: -1, viewportWidth: 0, compactPanel: null,
};
const listeners = new Set<() => void>();
const storageKey = (workspaceId: string | null) => `skarn:agents-workspace:v1:${workspaceId ?? 'none'}`;

export function clampAgentsWidth(panel: AgentsWorkspacePanel, value: number): number {
  const bounds = AGENTS_LAYOUT_BOUNDS[panel];
  return Math.min(bounds.max, Math.max(bounds.min, Number.isFinite(value) ? value : bounds.default));
}

function update(patch: Partial<AgentsWorkspaceState>, persist = false): void {
  state = { ...state, ...patch };
  if (persist) {
    try {
      localStorage.setItem(storageKey(state.workspaceId), JSON.stringify({
        navigationWidth: state.navigationWidth, contextWidth: state.contextWidth,
        navigationOpen: state.navigationOpen, contextOpen: state.contextOpen,
        lastContextTab: state.lastContextTab,
      }));
    } catch {
      // Restricted storage must not prevent using or resizing the workspace.
    }
  }
  listeners.forEach((listener) => listener());
}

export const agentsWorkspaceStore = {
  subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  getSnapshot: () => state,
  setWorkspace(workspaceId: string | null) {
    if (state.workspaceId === workspaceId) return;
    let saved: Partial<typeof defaults> = {};
    try {
      const value: unknown = JSON.parse(localStorage.getItem(storageKey(workspaceId)) ?? '{}');
      if (value && typeof value === 'object') saved = value as Partial<typeof defaults>;
    } catch {
      // Corrupt or unavailable storage falls back to bounded defaults.
    }
    update({
      workspaceId, page: 'conversation', history: [], historyIndex: -1, compactPanel: null,
      navigationWidth: clampAgentsWidth('navigation', typeof saved.navigationWidth === 'number' ? saved.navigationWidth : defaults.navigationWidth),
      contextWidth: clampAgentsWidth('context', typeof saved.contextWidth === 'number' ? saved.contextWidth : defaults.contextWidth),
      navigationOpen: typeof saved.navigationOpen === 'boolean' ? saved.navigationOpen : true,
      contextOpen: typeof saved.contextOpen === 'boolean' ? saved.contextOpen : true,
      lastContextTab: saved.lastContextTab === 'changes' ? 'changes' : 'files',
    });
  },
  resize(panel: AgentsWorkspacePanel, value: number) {
    update(panel === 'navigation' ? { navigationWidth: clampAgentsWidth(panel, value) } : { contextWidth: clampAgentsWidth(panel, value) }, true);
  },
  setViewportWidth(viewportWidth: number) {
    if (state.viewportWidth === viewportWidth) return;
    const compactPanel = state.compactPanel && viewportWidth < AGENTS_LAYOUT_BOUNDS[`${state.compactPanel}Breakpoint`] ? state.compactPanel : null;
    update({ viewportWidth, compactPanel });
  },
  closeCompactPanel() { update({ compactPanel: null }); },
  toggleNavigation() {
    if (state.viewportWidth < AGENTS_LAYOUT_BOUNDS.navigationBreakpoint) {
      update({ compactPanel: state.compactPanel === 'navigation' ? null : 'navigation' });
    } else update({ navigationOpen: !state.navigationOpen }, true);
  },
  toggleContext() {
    if (state.viewportWidth < AGENTS_LAYOUT_BOUNDS.contextBreakpoint) {
      update({ compactPanel: state.compactPanel === 'context' ? null : 'context' });
    } else update({ contextOpen: !state.contextOpen }, true);
  },
  setContextTab(lastContextTab: AgentsContextTab) {
    update({ lastContextTab, contextOpen: true,
      compactPanel: state.viewportWidth < AGENTS_LAYOUT_BOUNDS.contextBreakpoint ? 'context' : null }, true);
  },
  visit(visit: AgentsWorkspaceVisit) {
    const current = state.history[state.historyIndex];
    if (current && current.page === visit.page && current.sessionId === visit.sessionId && current.childId === visit.childId) return;
    const history = [...state.history.slice(0, state.historyIndex + 1), visit].slice(-100);
    update({ history, historyIndex: history.length - 1, page: visit.page });
  },
  historyTarget(delta: -1 | 1): AgentsWorkspaceVisit | null {
    return state.history[state.historyIndex + delta] ?? null;
  },
  commitHistory(delta: -1 | 1) {
    const index = state.historyIndex + delta;
    const visit = state.history[index];
    if (visit) update({ historyIndex: index, page: visit.page });
  },
};
