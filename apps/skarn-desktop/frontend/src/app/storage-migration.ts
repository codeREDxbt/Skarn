// Table-driven v0.1.0 -> v1.0.0 localStorage migration.
// Intentionally zero store imports so line-1 import in main.tsx runs before any eager store reads localStorage.

export type StorageKeyMapping =
  | { kind: 'exact'; from: string; to: string }
  | { kind: 'prefix'; from: string; to: string };

export const STORAGE_KEY_MIGRATIONS: ReadonlyArray<StorageKeyMapping> = [
  { kind: 'exact', from: 'selora:model_settings', to: 'skarn:model_settings' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'selora:agent-pinned-sessions', to: 'skarn:agent-pinned-sessions' }, // skarn-v1-migration-compat
  { kind: 'prefix', from: 'selora:agent-draft:', to: 'skarn:agent-draft:' }, // skarn-v1-migration-compat
  { kind: 'prefix', from: 'selora:editor-session:', to: 'skarn:editor-session:' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'selora:last-editor-workspace', to: 'skarn:last-editor-workspace' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'selora:editor-preferences', to: 'skarn:editor-preferences' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'selora:recent-commands', to: 'skarn:recent-commands' }, // skarn-v1-migration-compat
  { kind: 'prefix', from: 'selora:agents-workspace:v1:', to: 'skarn:agents-workspace:v1:' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'selora:ui-sounds', to: 'skarn:ui-sounds' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'selora:notification_history', to: 'skarn:notification_history' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'selora:onboarding_seen', to: 'skarn:onboarding_seen' }, // skarn-v1-migration-compat
  { kind: 'exact', from: 'agents:last-context-tab', to: 'skarn:agents-last-context-tab' }, // skarn-v1-migration-compat
];

function resolveDefaultStorage(): Storage | null {
  try {
    if (typeof window === 'undefined') return null;
    return window.localStorage ?? null;
  } catch {
    return null;
  }
}

export function migrateLegacyLocalStorage(storage?: Storage): number {
  const target = storage ?? resolveDefaultStorage();
  if (!target) return 0;

  let migrated = 0;
  try {
    const existingKeys: string[] = [];
    for (let i = 0; i < target.length; i += 1) {
      const key = target.key(i);
      if (key !== null) existingKeys.push(key);
    }

    for (const rule of STORAGE_KEY_MIGRATIONS) {
      if (rule.kind === 'exact') {
        const legacyValue = target.getItem(rule.from);
        if (legacyValue !== null && target.getItem(rule.to) === null) {
          target.setItem(rule.to, legacyValue);
          migrated += 1;
        }
        continue;
      }

      for (const oldKey of existingKeys) {
        if (!oldKey.startsWith(rule.from)) continue;
        const suffix = oldKey.slice(rule.from.length);
        const newKey = `${rule.to}${suffix}`;
        const legacyValue = target.getItem(oldKey);
        if (legacyValue !== null && target.getItem(newKey) === null) {
          target.setItem(newKey, legacyValue);
          migrated += 1;
        }
      }
    }
  } catch {
    return migrated;
  }

  return migrated;
}
