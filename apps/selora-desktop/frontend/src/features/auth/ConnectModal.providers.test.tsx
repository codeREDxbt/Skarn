// Connect modal preset filter: providers disabled in Settings > AI Gateway are
// hidden from the grid, while the current selection stays visible so a live
// connection is never silently switched.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => ({
  getApiKeyStatus: vi.fn(),
  connectApiKey: vi.fn(),
  disconnectApiKey: vi.fn(),
  fetchModels: vi.fn(),
  openExternal: vi.fn(),
}));

vi.mock('../../bridge/bridge', () => ({
  getApiKeyStatus: harness.getApiKeyStatus,
  connectApiKey: harness.connectApiKey,
  disconnectApiKey: harness.disconnectApiKey,
  fetchModels: harness.fetchModels,
  openExternal: harness.openExternal,
}));

import { ConnectModal } from './ConnectModal';

describe('ConnectModal assistant provider filter', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.getApiKeyStatus.mockResolvedValue({
      connected: false,
      has_key: false,
      source: 'none',
      base_url: '',
      key_hint: null,
      active_model_id: null,
    });
  });

  it('hides disabled providers and says where to re-enable them', async () => {
    const bootstrap = makeBootstrap();
    render(
      <ConnectModal
        open
        onClose={() => undefined}
        bootstrap={{
          ...bootstrap,
          settings: { ...bootstrap.settings, disabled_providers: ['deepseek', 'ollama'] },
        }}
      />,
    );

    await waitFor(() => expect(harness.getApiKeyStatus).toHaveBeenCalled());
    expect(screen.queryByRole('button', { name: /DeepSeek \(Direct\)/ })).toBeNull();
    expect(screen.queryByRole('button', { name: /Local Ollama \(Local\)/ })).toBeNull();
    expect(screen.getByRole('button', { name: /Selora API Gateway/ })).toBeTruthy();
    expect(screen.getByText(/2 providers hidden by Settings > AI Gateway/)).toBeTruthy();
  });

  it('keeps the currently selected provider visible even when disabled', async () => {
    const bootstrap = makeBootstrap();
    render(
      <ConnectModal
        open
        onClose={() => undefined}
        bootstrap={{
          ...bootstrap,
          settings: { ...bootstrap.settings, disabled_providers: ['selora'] },
        }}
      />,
    );

    await waitFor(() => expect(harness.getApiKeyStatus).toHaveBeenCalled());
    expect(screen.getByRole('button', { name: /Selora API Gateway/ })).toBeTruthy();
    expect(screen.queryByText(/hidden by Settings > AI Gateway/)).toBeNull();
  });
});