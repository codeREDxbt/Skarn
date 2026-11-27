// Workspace trust: the settings panel must surface the requirement, show the
// current grant, and persist a change instead of silently accepting it.
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

function bootstrapWithTrust(workspaceTrusted: boolean) {
  const bootstrap = makeBootstrap();
  return {
    ...bootstrap,
    settings: { ...bootstrap.settings, workspace_trusted: workspaceTrusted },
  };
}

describe('SettingsPanelView workspace trust', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.updateSettings.mockResolvedValue(undefined);
    harness.restartLsp.mockResolvedValue(undefined);
  });

  it('shows the trust requirement and the current restricted state', () => {
    render(<SettingsPanelView bootstrap={bootstrapWithTrust(false)} />);

    expect(screen.getByText('Workspace Trust')).toBeTruthy();
    expect(screen.getByText('Restricted')).toBeTruthy();
    expect(
      screen.getByText(/never run risky shell commands, even after you approve/i),
    ).toBeTruthy();
    expect(
      screen.getByText(/Read-only commands on the agent allowlist still run/i),
    ).toBeTruthy();
    const toggle = screen.getByLabelText(
      'Trust this workspace for risky agent commands',
    ) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
  });

  it('persists a trust grant through the settings patch', async () => {
    render(<SettingsPanelView bootstrap={bootstrapWithTrust(false)} />);

    fireEvent.click(
      screen.getByLabelText('Trust this workspace for risky agent commands'),
    );

    await waitFor(() =>
      expect(harness.updateSettings).toHaveBeenCalledWith(
        { workspace_trusted: true },
        expect.objectContaining({ core_instance_id: 'core-0001' }),
      ),
    );
  });

  it('shows a stored trust grant instead of rewriting it', () => {
    render(<SettingsPanelView bootstrap={bootstrapWithTrust(true)} />);

    expect(screen.getByText('Trusted')).toBeTruthy();
    const toggle = screen.getByLabelText(
      'Trust this workspace for risky agent commands',
    ) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    expect(harness.updateSettings).not.toHaveBeenCalled();
  });
});