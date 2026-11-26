import fs from 'node:fs';
import path from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { migrateLegacyLocalStorage, STORAGE_KEY_MIGRATIONS } from './storage-migration';

describe('localStorage migration', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('migrates all 12 keys cleanly while preserving parameterized suffixes and legacy keys', () => {
    const fixtures: Array<[string, string, string]> = [
      ['selora:model_settings', 'skarn:model_settings', '{"temperature":0.4}'], // skarn-v1-migration-compat
      ['selora:agent-pinned-sessions', 'skarn:agent-pinned-sessions', '["s1","s2"]'], // skarn-v1-migration-compat
      ['selora:agent-draft:["main","ws1","s1"]', 'skarn:agent-draft:["main","ws1","s1"]', '{"text":"draft","attachments":[],"revision":1}'], // skarn-v1-migration-compat
      ['selora:editor-session:c:/Work/Repo', 'skarn:editor-session:c:/Work/Repo', '{"files":[{"path":"a.ts","selection":null}],"activePath":"a.ts"}'], // skarn-v1-migration-compat
      ['selora:editor-session:/home/user/MyRepo', 'skarn:editor-session:/home/user/MyRepo', '{"files":[],"activePath":null}'], // skarn-v1-migration-compat
      ['selora:last-editor-workspace', 'skarn:last-editor-workspace', 'c:/Work/Repo'], // skarn-v1-migration-compat
      ['selora:editor-preferences', 'skarn:editor-preferences', '{"autoSave":true}'], // skarn-v1-migration-compat
      ['selora:recent-commands', 'skarn:recent-commands', '["selora.showFiles"]'], // skarn-v1-migration-compat
      ['selora:agents-workspace:v1:ws_abc', 'skarn:agents-workspace:v1:ws_abc', '{"navigationWidth":280}'], // skarn-v1-migration-compat
      ['selora:ui-sounds', 'skarn:ui-sounds', 'true'], // skarn-v1-migration-compat
      ['selora:notification_history', 'skarn:notification_history', '[{"id":1,"message":"hi"}]'], // skarn-v1-migration-compat
      ['selora:onboarding_seen', 'skarn:onboarding_seen', 'true'], // skarn-v1-migration-compat
      ['agents:last-context-tab', 'skarn:agents-last-context-tab', 'changes'], // skarn-v1-migration-compat
    ];

    for (const [oldKey, , value] of fixtures) {
      localStorage.setItem(oldKey, value);
    }

    const migrated = migrateLegacyLocalStorage();
    expect(migrated).toBe(fixtures.length);
    expect(STORAGE_KEY_MIGRATIONS).toHaveLength(12);

    for (const [oldKey, newKey, value] of fixtures) {
      expect(localStorage.getItem(newKey)).toBe(value);
      expect(localStorage.getItem(oldKey)).toBe(value);
    }
    expect(localStorage.getItem('skarn:onboarding_seen')).toBe('true');
  });

  it('does not overwrite existing skarn:* keys and is idempotent', () => {
    localStorage.setItem('selora:model_settings', '{"temperature":0.9}'); // skarn-v1-migration-compat
    localStorage.setItem('skarn:model_settings', '{"temperature":0.2}');
    localStorage.setItem('selora:onboarding_seen', 'false'); // skarn-v1-migration-compat
    localStorage.setItem('skarn:onboarding_seen', 'true');
    localStorage.setItem('selora:ui-sounds', 'true'); // skarn-v1-migration-compat

    const first = migrateLegacyLocalStorage();
    expect(first).toBe(1);
    expect(localStorage.getItem('skarn:model_settings')).toBe('{"temperature":0.2}');
    expect(localStorage.getItem('skarn:onboarding_seen')).toBe('true');
    expect(localStorage.getItem('skarn:ui-sounds')).toBe('true');

    const second = migrateLegacyLocalStorage();
    expect(second).toBe(0);
  });

  it('preserves edge-case suffixes and degrades safely when storage throws', () => {
    const colonKey = 'selora:editor-session://wsl$/Ubuntu/home/dev:repo'; // skarn-v1-migration-compat
    const jsonKey = 'selora:agent-draft:["agents","ws:1","sess:2"]'; // skarn-v1-migration-compat
    localStorage.setItem(colonKey, '{"files":[]}');
    localStorage.setItem(jsonKey, '{"text":"hello"}');

    expect(migrateLegacyLocalStorage()).toBe(2);
    expect(localStorage.getItem('skarn:editor-session://wsl$/Ubuntu/home/dev:repo')).toBe('{"files":[]}');
    expect(localStorage.getItem('skarn:agent-draft:["agents","ws:1","sess:2"]')).toBe('{"text":"hello"}');

    const throwingStorage: Storage = {
      get length(): number {
        throw new Error('storage blocked');
      },
      clear: () => {
        throw new Error('storage blocked');
      },
      getItem: () => {
        throw new Error('storage blocked');
      },
      key: () => {
        throw new Error('storage blocked');
      },
      removeItem: () => {
        throw new Error('storage blocked');
      },
      setItem: () => {
        throw new Error('storage blocked');
      },
    };
    expect(() => migrateLegacyLocalStorage(throwingStorage)).not.toThrow();
    expect(migrateLegacyLocalStorage(throwingStorage)).toBe(0);
  });

  it('runs before eager stores via line-1 main.tsx import without ESM hoisting leaks', async () => {
    localStorage.setItem('selora:onboarding_seen', 'true'); // skarn-v1-migration-compat
    localStorage.setItem('selora:ui-sounds', 'true'); // skarn-v1-migration-compat

    vi.resetModules();
    await import('./run-storage-migration');
    const { onboardingStore } = await import('../features/onboarding/onboarding-store');
    const soundStore = await import('../features/sound/sound-store');

    expect(onboardingStore.shouldShow()).toBe(false);
    expect(soundStore.getSnapshot()).toBe(true);

    const mainSource = fs.readFileSync(path.resolve(__dirname, '../main.tsx'), 'utf8');
    const firstNonEmptyLine = mainSource
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line.length > 0);
    expect(firstNonEmptyLine).toBe("import './app/run-storage-migration';");

    const surfaceSource = fs.readFileSync(path.resolve(__dirname, './surface.ts'), 'utf8');
    expect(mainSource).not.toMatch(/from\s+['"][^'"]*(store|local-preferences)['"]/);
    expect(surfaceSource).not.toMatch(/from\s+['"][^'"]*(store|local-preferences)['"]/);
  });
});
