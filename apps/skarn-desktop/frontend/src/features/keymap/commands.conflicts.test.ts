// Keymap conflict coverage: the settings panel warns before it persists a
// rebind that would collide with a fixed chord or another rebindable one.
import { describe, expect, it } from 'vitest';
import { buildCommandList, validateKeymapConflicts } from './commands';
import { makeBootstrap } from '../../testing/fixtures';

const SETTINGS = makeBootstrap().settings;

describe('validateKeymapConflicts', () => {
  it('keeps every default chord unique', () => {
    const shortcuts = buildCommandList(SETTINGS)
      .map((command) => command.shortcut)
      .filter((shortcut): shortcut is string => shortcut !== null);
    expect(new Set(shortcuts).size).toBe(shortcuts.length);
    expect(validateKeymapConflicts(SETTINGS)).toEqual([]);
  });

  it('reports a rebind that collides with a fixed chord', () => {
    expect(validateKeymapConflicts({ ...SETTINGS, keybind_save: 'Ctrl+B' })).toEqual([
      {
        shortcut: 'Ctrl+B',
        commands: [
          { id: 'selora.togglePrimarySidebar', title: 'Toggle Primary Sidebar' },
          { id: 'selora.fileSave', title: 'Save' },
        ],
      },
    ]);
  });

  it('reports two rebindable commands sharing one chord', () => {
    const conflicts = validateKeymapConflicts({
      ...SETTINGS,
      keybind_command_palette: 'Ctrl+P',
    });
    expect(conflicts).toHaveLength(1);
    expect(conflicts[0].shortcut).toBe('Ctrl+P');
    expect(conflicts[0].commands.map((command) => command.id)).toEqual([
      'selora.openCommandPalette',
      'selora.editFindInFiles',
    ]);
  });

  it('ignores commands without a shortcut', () => {
    const conflicts = validateKeymapConflicts(SETTINGS);
    const unbound = buildCommandList(SETTINGS).filter((command) => command.shortcut === null);
    expect(unbound.length).toBeGreaterThan(0);
    expect(conflicts.some((conflict) => conflict.shortcut === '')).toBe(false);
  });
});