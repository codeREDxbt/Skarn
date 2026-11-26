// Settings > Providers "Assistant providers": the toggles persist the
// disabled list immediately, a disabled provider leaves the selection lists,
// and a live disabled provider is reported without switching the model.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => ({
  updateSettings: vi.fn(),
  restartLsp: vi.fn(),
}));

vi.mock('../../bridge/bridge', () => ({
  updateSettings: harness.updateSettings,
  restartLsp: harness.restartLsp,
}));

import { SettingsPanelView } from './SettingsPanelView';

describe('SettingsPanelView assistant providers', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.updateSettings.mockResolvedValue(undefined);
  });

  it('lists every provider with an enable toggle', () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    expect(screen.getByText('Assistant providers')).toBeTruthy();
    expect(screen.getByText('7 of 7 enabled')).toBeTruthy();
    expect(screen.queryByLabelText('Enable Selora API Gateway')).toBeNull();
    expect(screen.getByLabelText('Enable Anthropic Claude')).toBeTruthy();
    expect(screen.getByLabelText('Enable DeepSeek')).toBeTruthy();
    expect(screen.getByLabelText('Enable Custom Provider')).toBeTruthy();
  });

  it('persists a disable immediately and posts only the disabled list', async () => {
    render(<SettingsPanelView bootstrap={makeBootstrap()} />);

    fireEvent.click(screen.getByLabelText('Enable DeepSeek'));

    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { disabled_providers: ['deepseek'] },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );
  });

  it('re-enables a disabled provider from the same toggle', async () => {
    const bootstrap = makeBootstrap();
    render(
      <SettingsPanelView
        bootstrap={{
          ...bootstrap,
          settings: { ...bootstrap.settings, disabled_providers: ['deepseek', 'ollama'] },
        }}
      />,
    );

    expect(screen.getByText('5 of 7 enabled')).toBeTruthy();
    const toggle = screen.getByLabelText('Enable DeepSeek') as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { disabled_providers: ['ollama'] },
        expect.anything(),
      ),
    );
  });

  it('warns when the active provider is disabled and keeps the current model', () => {
    const bootstrap = makeBootstrap();
    render(
      <SettingsPanelView
        bootstrap={{
          ...bootstrap,
          models: [{ id: 'deepseek-chat', display_name: 'DeepSeek Chat', available: true }],
          active_model_id: 'deepseek-chat',
          settings: { ...bootstrap.settings, disabled_providers: ['deepseek'] },
        }}
      />,
    );

    expect(screen.getByRole('alert').textContent).toContain(
      'DeepSeek is disabled but remains active for this session.',
    );
    expect(screen.getByRole('alert').textContent).toContain(
      'Current model DeepSeek Chat is unchanged.',
    );
    // The model field still shows the live model; no switch was attempted.
    const model = screen.getByLabelText('Chat Model') as HTMLSelectElement;
    expect(model.value).toBe('deepseek-chat');
    expect(harness.updateSettings).not.toHaveBeenCalled();
  });

  it('filters disabled providers out of the provider selection list', () => {
    const bootstrap = makeBootstrap();
    render(
      <SettingsPanelView
        bootstrap={{
          ...bootstrap,
          models: [{ id: 'claude-sonnet-4', display_name: 'Claude Sonnet 4', available: true }],
          active_model_id: 'claude-sonnet-4',
          settings: { ...bootstrap.settings, disabled_providers: ['deepseek', 'ollama'] },
        }}
      />,
    );

    const provider = screen.getByLabelText('Default Provider') as HTMLSelectElement;
    const values = Array.from(provider.options).map((option) => option.value);
    expect(values).toContain('anthropic');
    expect(values).not.toContain('deepseek');
    expect(values).not.toContain('ollama');
    expect(provider.value).toBe('anthropic');
  });
});