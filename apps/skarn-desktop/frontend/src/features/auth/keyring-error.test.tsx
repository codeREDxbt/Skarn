import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ApiKeyStatus, EventEnvelope } from '../../bridge/bridge';
import { makeBootstrap, makeEvent } from '../../testing/fixtures';

const harness = vi.hoisted(() => ({
  getBootstrapState: vi.fn(),
  getEventsAfter: vi.fn(),
  getAgentsSnapshot: vi.fn(),
  listenProtocolEvent: vi.fn(),
  markFrontendReady: vi.fn(),
  getApiKeyStatus: vi.fn(),
  connectApiKey: vi.fn(),
  disconnectApiKey: vi.fn(),
  fetchModels: vi.fn(),
  openExternal: vi.fn(),
  listener: null as ((event: EventEnvelope) => void) | null,
}));

vi.mock('../../bridge/bridge', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../bridge/bridge')>();
  return {
    ...actual,
    getBootstrapState: harness.getBootstrapState,
    getEventsAfter: harness.getEventsAfter,
    getAgentsSnapshot: harness.getAgentsSnapshot,
    listenProtocolEvent: harness.listenProtocolEvent,
    markFrontendReady: harness.markFrontendReady,
    getApiKeyStatus: harness.getApiKeyStatus,
    connectApiKey: harness.connectApiKey,
    disconnectApiKey: harness.disconnectApiKey,
    fetchModels: harness.fetchModels,
    openExternal: harness.openExternal,
  };
});

vi.mock('../../app/surface', () => ({
  isNativeFrontend: () => true,
  getFrontendSurface: () => ({ kind: 'main', id: 'main' }),
}));

describe('keyring failure surfacing across app-store, StatusBar, and ConnectModal', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.listener = null;
    harness.markFrontendReady.mockResolvedValue(undefined);
    harness.getEventsAfter.mockResolvedValue([]);
    harness.getAgentsSnapshot.mockResolvedValue({
      protocol_version: 1,
      core_instance_id: 'core-0001',
      workspace_id: 'no-workspace',
      snapshot_sequence: 0,
      sessions: [],
      active_session_id: null,
      active_request_id: null,
      active_turn_id: null,
    });
    harness.listenProtocolEvent.mockImplementation(async (cb: (event: EventEnvelope) => void) => {
      harness.listener = cb;
      return () => {
        harness.listener = null;
      };
    });
  });

  it('hydrates connectionError from initial getApiKeyStatus and renders StatusBar prompt even without a live event', async () => {
    vi.resetModules();
    const bootstrap = makeBootstrap({ connection: 'disconnected' });
    harness.getBootstrapState.mockResolvedValue(bootstrap);
    const keyringErrStatus: ApiKeyStatus = {
      connected: false,
      has_key: false,
      source: 'keyring_error',
      base_url: '',
      key_hint: null,
      active_model_id: null,
      last_error: 'The OS keyring could not be read: vault locked',
    };
    harness.getApiKeyStatus.mockResolvedValue(keyringErrStatus);

    const appStore = await import('../../app/app-store');
    const { StatusBar } = await import('../../layout/StatusBar');

    await appStore.startApp();

    await waitFor(() => {
      expect(appStore.getAppSnapshot().connectionError).toBe(
        'The OS keyring could not be read: vault locked',
      );
    });

    const onOpenConnect = vi.fn();
    render(<StatusBar bootstrap={bootstrap} onOpenConnect={onOpenConnect} />);

    const promptBtn = screen.getByRole('button', { name: /keyring error · re-enter key/i });
    expect(promptBtn).toBeTruthy();
    fireEvent.click(promptBtn);
    expect(onOpenConnect).toHaveBeenCalledOnce();
  });

  it('updates and clears connectionError on live skarn_status_changed events', async () => {
    vi.resetModules();
    const bootstrap = makeBootstrap({ connection: 'disconnected' });
    harness.getBootstrapState.mockResolvedValue(bootstrap);
    harness.getApiKeyStatus.mockResolvedValue({
      connected: false,
      has_key: false,
      source: 'none',
      base_url: '',
      key_hint: null,
      active_model_id: null,
      last_error: null,
    });

    const appStore = await import('../../app/app-store');
    await appStore.startApp();
    await waitFor(() => expect(harness.listener).not.toBeNull());

    harness.listener!(
      makeEvent(1, {
        type: 'skarn_status_changed',
        payload: {
          status: 'disconnected',
          active_model_id: null,
          error: 'The OS keyring could not be read: denied',
        },
      }),
    );
    expect(appStore.getAppSnapshot().connectionError).toBe(
      'The OS keyring could not be read: denied',
    );

    harness.listener!(
      makeEvent(2, {
        type: 'skarn_status_changed',
        payload: {
          status: 'connected',
          active_model_id: 'model-1',
          error: null,
        },
      }),
    );
    expect(appStore.getAppSnapshot().connectionError).toBeNull();
  });

  it('renders the actionable Windows Credential Manager re-entry prompt in ConnectModal', async () => {
    vi.resetModules();
    harness.getApiKeyStatus.mockResolvedValue({
      connected: false,
      has_key: false,
      source: 'keyring_error',
      base_url: 'https://api.openai.com/v1',
      key_hint: null,
      active_model_id: null,
      last_error: 'The OS keyring could not be read: vault locked',
    });

    const { ConnectModal } = await import('./ConnectModal');
    render(
      <ConnectModal
        open={true}
        onClose={() => undefined}
        bootstrap={makeBootstrap({ connection: 'disconnected' })}
      />,
    );

    await waitFor(() => {
      expect(screen.getByRole('alert').textContent).toContain(
        'Saved key could not be read from Windows Credential Manager — please re-enter your API key.',
      );
    });
  });
});
