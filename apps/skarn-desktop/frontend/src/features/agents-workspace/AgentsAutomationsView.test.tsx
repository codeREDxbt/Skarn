import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { workflowsStore } from '../workflows/workflows-store';
import { AgentsAutomationsView } from './AgentsAutomationsView';

describe('AgentsAutomationsView', () => {
  beforeEach(() => workflowsStore.reset());

  it('renders the real empty workflow state without inventing automations', () => {
    render(<AgentsAutomationsView />);
    expect(screen.getByRole('heading', { name: 'Automations' })).toBeTruthy();
    expect(screen.getByTestId('workflows-empty').textContent).toContain('No workflow runs');
    expect(screen.queryByRole('button', { name: 'Stop' })).toBeNull();
  });

  it('preserves real store updates, approval decisions, and stop actions', async () => {
    const stop = vi.fn().mockResolvedValue(undefined);
    const approve = vi.fn().mockResolvedValue(undefined);
    workflowsStore.setStopHandler(stop);
    workflowsStore.setApprovalHandler(approve);
    render(<AgentsAutomationsView store={workflowsStore} />);
    act(() => {
      workflowsStore.applyEvent({ kind: 'started', run_id: 'run-8', name: 'Real workflow', phases: ['Review'] });
      workflowsStore.applyEvent({ kind: 'approval', run_id: 'run-8', approval_id: 'approval-8', summary: 'Write report', state: 'pending' });
    });
    expect(screen.getByTestId('workflows-panel')).toBeTruthy();
    expect(screen.queryByTestId('workflows-empty')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Approve Write report' }));
    await waitFor(() => expect(approve).toHaveBeenCalledWith('run-8', 'approval-8', 'approve'));
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(stop).toHaveBeenCalledWith('run-8'));
  });
});
