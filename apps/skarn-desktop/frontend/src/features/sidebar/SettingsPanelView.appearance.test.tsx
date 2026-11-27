// Appearance + keymap wiring for the settings panel: the system theme option,
// font-scale validation, and the conflict warning all stop at the UI unless
// the value is safe to persist.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => ({
  updateSettings: vi.fn(),
  restartLsp: vi.fn(),
  revokeScopedGrant: vi.fn(),
}));

vi.mock('../../bridge/bridge', () => ({
  updateSettings: harness.updateSettings,
  restartLsp: harness.restartLsp,
  revokeScopedGrant: harness.revokeScopedGrant,
}));

import { SettingsPanelView } from './SettingsPanelView';

describe('SettingsPanelView appearance and keymap', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.updateSettings.mockResolvedValue(undefined);
    harness.restartLsp.mockResolvedValue(undefined);
  });

  it('offers the system theme and persists it as the stored preference', async () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    const theme = screen.getByLabelText('Color Theme') as HTMLSelectElement;
    expect(Array.from(theme.options).map((option) => option.value)).toEqual([
      'skarn-dark',
      'galaxy',
      'dark',
      'light',
      'system',
    ]);
    expect(screen.getByText('System follows your operating system appearance.')).toBeTruthy();

    fireEvent.change(theme, { target: { value: 'system' } });
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { theme: 'system' },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );
  });

  it('shows a stored system preference instead of rewriting it', () => {
    const bootstrap = makeBootstrap();
    render(
      <SettingsPanelView
        bootstrap={{ ...bootstrap, settings: { ...bootstrap.settings, theme: 'system' } }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Theme' }));
    expect((screen.getByLabelText('Color Theme') as HTMLSelectElement).value).toBe('system');
  });

  it('persists a valid font size so the live scale can follow it', async () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Editor' }));
    const font = screen.getByLabelText('Font Size (px)') as HTMLInputElement;
    expect(font.min).toBe('8');
    expect(font.max).toBe('32');

    fireEvent.change(font, { target: { value: '22' } });
    fireEvent.blur(font);
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { editor_font_size: 22 },
        expect.anything(),
      ),
    );
  });

  it('keeps an out-of-range font size local and never persists it', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Editor' }));
    const font = screen.getByLabelText('Font Size (px)');
    fireEvent.change(font, { target: { value: '99' } });
    fireEvent.blur(font);

    expect(screen.getByRole('alert').textContent).toContain(
      'Font size must be a whole number from 8 to 32.',
    );
    expect(harness.updateSettings).not.toHaveBeenCalled();
  });

  it('warns about a keymap conflict and does not persist the rebind', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByRole('button', { name: 'Keymap' }));
    const save = screen.getByLabelText('Save') as HTMLInputElement;
    expect(save.value).toBe('Ctrl+S');

    fireEvent.change(save, { target: { value: 'Ctrl+B' } });
    fireEvent.blur(save);

    expect(screen.getByRole('alert').textContent).toContain(
      'Shortcut conflict: Ctrl+B is assigned to Toggle Primary Sidebar and Save.',
    );
    expect(harness.updateSettings).not.toHaveBeenCalled();
  });
});