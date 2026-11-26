import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiKeyStatus, Model } from '../../bridge/bridge';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => ({
  connectApiKey: vi.fn(),
  disconnectApiKey: vi.fn(),
  getApiKeyStatus: vi.fn(),
  fetchModels: vi.fn(),
  openExternal: vi.fn(),
  refreshBootstrap: vi.fn(),
}));

vi.mock('../../bridge/bridge', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../bridge/bridge')>();
  return {
    ...actual,
    connectApiKey: harness.connectApiKey,
    disconnectApiKey: harness.disconnectApiKey,
    getApiKeyStatus: harness.getApiKeyStatus,
    fetchModels: harness.fetchModels,
    openExternal: harness.openExternal,
  };
});

vi.mock('../../app/app-store', () => ({
  currentEnvelopeContext: () => ({
    workspace_id: 'ws-0001',
    core_instance_id: 'core-0001',
    session_id: null,
  }),
  refreshBootstrap: harness.refreshBootstrap,
}));

import { agentStore } from '../agent/agent-store';
import { ConnectModal } from './ConnectModal';

const MODEL_A: Model = {
  id: 'first/model',
  display_name: 'First Model',
  available: true,
  tool_capability: 'supported',
};

const MODEL_B: Model = {
  id: 'mistralai/mistral-large-4-1t-a52b',
  display_name: 'Mistral Large 4',
  available: true,
  tool_capability: 'supported',
};

describe('ConnectModal truthful discovery & selection preservation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentStore.reset();
    localStorage.clear();
    harness.refreshBootstrap.mockResolvedValue(undefined);
  });

  it('shows model_discovery_failed error and does not report Connected on connect failure', async () => {
    const disconnectedStatus: ApiKeyStatus = {
      connected: false,
      has_key: false,
      source: 'none',
      base_url: 'https://integrate.api.nvidia.com/v1',
      key_hint: null,
      active_model_id: null,
      last_error: null,
    };
    harness.getApiKeyStatus.mockResolvedValue(disconnectedStatus);
    harness.connectApiKey.mockRejectedValue({
      code: 'model_discovery_failed',
      message: 'Could not discover models from NVIDIA (integrate.api.nvidia.com): HTTP 401 unauthorized.',
      retryable: false,
    });

    render(
      <ConnectModal
        open={true}
        onClose={vi.fn()}
        bootstrap={makeBootstrap({ connection: 'disconnected', models: [] })}
      />,
    );

    const keyInput = await screen.findByLabelText(/^API Key/);
    fireEvent.change(keyInput, { target: { value: 'nvapi-bad-secret' } });
    fireEvent.click(screen.getByRole('button', { name: /Test & Connect/i }));

    await waitFor(() => {
      expect(screen.getByRole('alert').textContent).toContain(
        'Could not discover models from NVIDIA (integrate.api.nvidia.com): HTTP 401 unauthorized.',
      );
    });
    expect(screen.getByText('Gateway Not Connected')).toBeTruthy();
    expect(screen.queryByText('Gateway Connected')).toBeNull();
  });

  it('preserves explicit selectedModel across reconnect and catalog refresh', async () => {
    agentStore.hydrate(
      makeBootstrap({
        connection: 'connected',
        models: [MODEL_A, MODEL_B],
        active_model_id: MODEL_B.id,
      }),
    );

    const connectedStatus: ApiKeyStatus = {
      connected: true,
      has_key: true,
      source: 'keyring',
      base_url: 'https://integrate.api.nvidia.com/v1',
      key_hint: 'nvap••••key1',
      active_model_id: MODEL_B.id,
      last_error: null,
    };
    harness.getApiKeyStatus.mockResolvedValue(connectedStatus);
    harness.connectApiKey.mockResolvedValue({
      connected: true,
      models_count: 2,
      active_model_id: MODEL_B.id,
      base_url: 'https://integrate.api.nvidia.com/v1',
    });
    harness.fetchModels.mockResolvedValue([MODEL_A, MODEL_B]);

    render(
      <ConnectModal
        open={true}
        onClose={vi.fn()}
        bootstrap={makeBootstrap({
          connection: 'connected',
          models: [MODEL_A, MODEL_B],
          active_model_id: MODEL_B.id,
        })}
      />,
    );

    // Reconnect with new credential: must preserve MODEL_B.id.
    const keyInput = await screen.findByLabelText(/^API Key/);
    fireEvent.change(keyInput, { target: { value: 'nvapi-updated-key' } });
    fireEvent.click(screen.getByRole('button', { name: /Test & Connect/i }));

    await waitFor(() => {
      expect(harness.connectApiKey).toHaveBeenCalled();
    });
    expect(agentStore.getSnapshot().selectedModel).toBe(MODEL_B.id);

    // Switch to Model & Reasoning Settings tab and refresh catalog.
    fireEvent.click(screen.getByRole('tab', { name: /Model & Reasoning Settings/i }));
    fireEvent.click(screen.getByRole('button', { name: /Refresh Models/i }));

    await waitFor(() => {
      expect(harness.fetchModels).toHaveBeenCalled();
    });
    expect(agentStore.getSnapshot().selectedModel).toBe(MODEL_B.id);
    const select = document.getElementById('model-select') as HTMLSelectElement;
    expect(select.value).toBe(MODEL_B.id);
  });
});
