import { beforeEach, describe, expect, it } from 'vitest';
import { AGENTS_LAYOUT_BOUNDS, agentsWorkspaceStore as store, clampAgentsWidth } from './agents-workspace-store';

let workspace = 0;
beforeEach(() => {
  localStorage.clear();
  store.setWorkspace(`workspace-${++workspace}`);
  store.setViewportWidth(1400);
});

describe('Agents workspace layout', () => {
  it('bounds corrupt and non-finite persisted sizes', () => {
    localStorage.setItem('skarn:agents-workspace:v1:corrupt', JSON.stringify({ navigationWidth: -100, contextWidth: 99999, navigationOpen: 'false' }));
    store.setWorkspace('corrupt');
    expect(store.getSnapshot().navigationWidth).toBe(AGENTS_LAYOUT_BOUNDS.navigation.min);
    expect(store.getSnapshot().contextWidth).toBe(AGENTS_LAYOUT_BOUNDS.context.max);
    expect(store.getSnapshot().navigationOpen).toBe(true);
    expect(clampAgentsWidth('navigation', NaN)).toBe(AGENTS_LAYOUT_BOUNDS.navigation.default);
  });

  it('persists widths per workspace without touching editor layout', () => {
    store.setWorkspace('one');
    store.resize('navigation', 300);
    store.resize('context', 410);
    store.toggleNavigation();
    store.setWorkspace('two');
    expect(store.getSnapshot().navigationWidth).toBe(AGENTS_LAYOUT_BOUNDS.navigation.default);
    store.setWorkspace('one');
    expect(store.getSnapshot()).toMatchObject({ navigationWidth: 300, contextWidth: 410, navigationOpen: false });
    expect(Object.keys(localStorage)).not.toContain('skarn:layout');
  });

  it('uses compact drawers without overwriting saved desktop widths', () => {
    store.resize('navigation', 300);
    store.setViewportWidth(600);
    store.toggleNavigation();
    expect(store.getSnapshot()).toMatchObject({ compactPanel: 'navigation', navigationWidth: 300, navigationOpen: true });
    store.setContextTab('changes');
    expect(store.getSnapshot()).toMatchObject({ compactPanel: 'context', lastContextTab: 'changes' });
    store.setViewportWidth(1400);
    expect(store.getSnapshot()).toMatchObject({ compactPanel: null, navigationWidth: 300 });
  });
});

describe('Agents visit history', () => {
  const visit = (sessionId: string, page: 'conversation' | 'automations' = 'conversation') => ({ page, sessionId, childId: null });
  it('records actual page/session visits, deduplicates, and truncates forward history', () => {
    store.visit(visit('a'));
    store.visit(visit('a'));
    store.visit(visit('b'));
    store.visit(visit('b', 'automations'));
    expect(store.getSnapshot().history).toHaveLength(3);
    expect(store.historyTarget(-1)).toEqual(visit('b'));
    store.commitHistory(-1);
    store.visit({ ...visit('b'), childId: 'child' });
    expect(store.historyTarget(1)).toBeNull();
    expect(store.getSnapshot().history.at(-1)?.childId).toBe('child');
  });

  it('invalidates history when workspace changes', () => {
    store.visit(visit('a'));
    store.setWorkspace('another');
    expect(store.getSnapshot()).toMatchObject({ history: [], historyIndex: -1, page: 'conversation' });
  });
});
