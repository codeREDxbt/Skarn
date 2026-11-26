import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useShortcuts } from './use-shortcuts';

afterEach(cleanup);

describe('Agents shortcuts', () => {
  function Harness({ scope, onCommand }: { scope: 'editor' | 'agents'; onCommand: ReturnType<typeof vi.fn> }) {
    useShortcuts({ settings: null, enabled: true, paletteOpen: false, onCommand, onClosePalette: () => undefined, scope });
    return <textarea aria-label="Composer" />;
  }

  it('opens the native Agents window while typing without consuming the old proposed chord', () => {
    const onCommand = vi.fn();
    const { getByRole } = render(<Harness scope="editor" onCommand={onCommand} />);
    fireEvent.keyDown(getByRole('textbox'), { key: 'A', ctrlKey: true, shiftKey: true, altKey: true });
    expect(onCommand).toHaveBeenCalledWith('selora.openAgentsWindow');
    onCommand.mockClear();
    fireEvent.keyDown(getByRole('textbox'), { key: 'A', ctrlKey: true, shiftKey: true });
    expect(onCommand).not.toHaveBeenCalled();
  });

  it('makes Ctrl+N live in Agents composer only', () => {
    const onCommand = vi.fn();
    const { getByRole, rerender } = render(<Harness scope="agents" onCommand={onCommand} />);
    fireEvent.keyDown(getByRole('textbox'), { key: 'n', ctrlKey: true });
    expect(onCommand).toHaveBeenCalledWith('selora.agentsNewSession');
    onCommand.mockClear();
    rerender(<Harness scope="editor" onCommand={onCommand} />);
    fireEvent.keyDown(getByRole('textbox'), { key: 'n', ctrlKey: true });
    expect(onCommand).not.toHaveBeenCalled();
  });
});
