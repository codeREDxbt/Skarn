import { describe, expect, it, vi } from 'vitest';
import { buildCommandList, resolveKeymap, runCommandAction } from './commands';
import { parseKeybind } from './keybind';

describe('Agents command scope', () => {
  it('uses the requested global chord and preserves editor sidebar binding', () => {
    const editor = resolveKeymap(null);
    expect(editor.get('selora.openAgentsWindow')).toEqual(parseKeybind('Ctrl+Shift+Alt+A'));
    expect(editor.get('selora.toggleAgentDock')).toEqual(parseKeybind('Ctrl+L'));
    expect(editor.has('selora.agentsNewSession')).toBe(false);
    expect(editor.get('selora.togglePrimarySidebar')).toEqual(parseKeybind('Ctrl+B'));
  });

  it('keeps editor commands out of the focused Agents palette', () => {
    const commands = buildCommandList(null, 'agents');
    const ids = commands.map((command) => command.id);
    expect(ids).toContain('selora.agentsNewSession');
    expect(ids).toContain('selora.agentsShowFiles');
    expect(ids).toContain('selora.agentsShowChanges');
    expect(ids).toContain('selora.agentsReturnEditor');
    expect(ids).not.toContain('selora.fileSave');
    expect(ids).not.toContain('selora.toggleAgentDock');
    const shortcuts = commands.flatMap((command) => command.shortcut ? [command.shortcut] : []);
    expect(new Set(shortcuts).size).toBe(shortcuts.length);
    expect(resolveKeymap(null, 'agents').get('selora.agentsNewSession')).toEqual(parseKeybind('Ctrl+N'));
  });

  it('routes Agents actions without mutating editor surfaces', () => {
    const agentsCommand = vi.fn();
    const toggle = vi.fn();
    runCommandAction('selora.agentsShowChanges', {
      layout: () => null, toggle, saveLayout: vi.fn(), openPalette: vi.fn(), agentsCommand,
    });
    expect(agentsCommand).toHaveBeenCalledWith('show-changes');
    expect(toggle).not.toHaveBeenCalled();
  });
});
