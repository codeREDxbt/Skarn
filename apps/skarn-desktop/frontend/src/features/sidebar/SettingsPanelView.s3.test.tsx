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

describe('SettingsPanelView Stage 3 local_only and scoped grants', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.updateSettings.mockResolvedValue(undefined);
    harness.restartLsp.mockResolvedValue(undefined);
    harness.revokeScopedGrant.mockResolvedValue({ acknowledged: true });
  });

  it('dispatches global local_only, workspace override, and clear_workspace_local_only patches', async () => {
    const base = makeBootstrap();
    const { rerender } = render(
      <SettingsPanelView
        bootstrap={{
          ...base,
          settings: {
            ...base.settings,
            local_only: false,
            workspace_local_only: null,
            effective_local_only: false,
          },
        }}
      />,
    );

    expect(screen.getByTestId('effective-local-only-status').textContent).toBe('Cloud allowed');

    fireEvent.click(screen.getByLabelText('Global local-only mode'));
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { local_only: true },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );

    const select = screen.getByLabelText('Workspace local-only override') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'true' } });
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { workspace_local_only: true },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );

    rerender(
      <SettingsPanelView
        bootstrap={{
          ...base,
          settings: {
            ...base.settings,
            local_only: false,
            workspace_local_only: true,
            effective_local_only: true,
          },
        }}
      />,
    );

    expect(screen.getByTestId('effective-local-only-status').textContent).toBe('Local-only active');
    fireEvent.click(screen.getByRole('button', { name: 'Clear override' }));
    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { clear_workspace_local_only: true },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );
  });

  it('renders active scoped grants and dispatches one-click revocation', async () => {
    const base = makeBootstrap({
      scoped_grants: [
        {
          action_class: 'file_edit',
          target_pattern: 'src/*.ts',
          decision: 'allow_project',
          max_risk: 'medium',
        },
        {
          action_class: 'command_gated',
          target_pattern: 'rm -rf *',
          decision: 'deny_session',
          max_risk: 'high',
        },
      ],
    });

    render(<SettingsPanelView bootstrap={base} />);

    expect(screen.getByText('2 active')).toBeTruthy();
    expect(screen.getByText('src/*.ts')).toBeTruthy();
    expect(screen.getByText('rm -rf *')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Revoke file_edit src/*.ts' }));
    await waitFor(() =>
      expect(harness.revokeScopedGrant).toHaveBeenCalledWith(
        'file_edit',
        'src/*.ts',
        expect.objectContaining({ core_instance_id: 'core-0001', session_id: null }),
      ),
    );
  });
});
