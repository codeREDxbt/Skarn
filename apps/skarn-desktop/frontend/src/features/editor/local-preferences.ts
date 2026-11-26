export type LocalEditorPreferences = { autoSave: boolean; autoSaveDelay: number; restoreSession: boolean };
const KEY = 'skarn:editor-preferences';
const defaults: LocalEditorPreferences = { autoSave: false, autoSaveDelay: 1000, restoreSession: true };
const listeners = new Set<() => void>();

function load(): LocalEditorPreferences {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) ?? '{}');
    return {
      autoSave: value?.autoSave === true,
      autoSaveDelay: Number.isInteger(value?.autoSaveDelay) && value.autoSaveDelay >= 500 && value.autoSaveDelay <= 60000
        ? value.autoSaveDelay : defaults.autoSaveDelay,
      restoreSession: value?.restoreSession !== false,
    };
  } catch { return { ...defaults }; }
}
let state = load();

export const localEditorPreferences = {
  getSnapshot: () => state,
  subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  update(patch: Partial<LocalEditorPreferences>) {
    const next = { ...state, ...patch };
    if (!Number.isInteger(next.autoSaveDelay) || next.autoSaveDelay < 500 || next.autoSaveDelay > 60000) return;
    try { localStorage.setItem(KEY, JSON.stringify(next)); }
    catch { return; }
    state = next;
    for (const listener of listeners) listener();
  },
};
