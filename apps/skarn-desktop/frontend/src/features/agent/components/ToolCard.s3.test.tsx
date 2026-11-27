import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { makeBootstrap } from '../../../testing/fixtures';
import { agentStore, type WorkflowToolCall } from '../agent-store';
import { ToolCard } from './ToolCard';
import { DecisionTraceDrawer } from './DecisionTraceDrawer';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const SESSION_ID = 'sess-tool-s3';

function seedSessionWithApproval(
  risk: 'low' | 'medium' | 'high' | 'critical',
  toolName = 'edit_file',
  target = 'src/app.ts',
): WorkflowToolCall {
  agentStore.reset();
  agentStore.hydrate(
    makeBootstrap({
      workspace: { id: 'ws-1', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
      models: [{ id: 'm1', display_name: 'Model 1', available: true, tool_capability: 'supported' }],
      active_model_id: 'm1',
      chat: {
        sessions: [{ session_id: SESSION_ID, title: 'Tool S3', active: true, message_ids: [] }],
        messages: [{ message_id: 'm-user', session_id: SESSION_ID, role: 'user', text: 'Go' }],
        active_session_id: SESSION_ID,
        active_request_id: 'req-1',
        active_turn_id: 'turn-1',
        pending_approval: null,
      },
    }),
  );

  act(() => {
    agentStore.handleApprovalNeeded({
      request_id: 'req-1',
      session_id: SESSION_ID,
      turn_id: 'turn-1',
      approval_id: 'appr-1',
      tool_name: toolName,
      path_or_command: target,
      risk,
      action_class: risk === 'high' || risk === 'critical' ? 'file_delete' : 'file_edit',
    });
  });

  const lastMsg = agentStore.getSnapshot().messages[agentStore.getSnapshot().messages.length - 1]!;
  return lastMsg.toolCalls![0] as WorkflowToolCall;
}

describe('ToolCard scoped grants, risk ceiling & DecisionTraceDrawer', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue({ acknowledged: true });
  });

  it('dispatches scoped grants for medium-risk actions (allow_session / allow_project / deny_*)', async () => {
    const tool = seedSessionWithApproval('medium', 'edit_file', 'src/app.ts');
    render(<ToolCard tool={tool} />);

    expect(screen.getByTestId('tool-risk-badge').textContent).toBe('medium');
    expect(screen.getByRole('button', { name: 'Allow once' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Always allow (session)' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Always allow (project)' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Deny once' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Always deny (session)' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Always deny (project)' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Always allow (session)' }));

    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('dispatch_command', {
        envelope: expect.objectContaining({
          payload: {
            type: 'agent_decision',
            payload: {
              request_id: 'req-1',
              approval_id: 'appr-1',
              decision: 'approve',
              resolution: {
                kind: 'scoped',
                grant: 'allow_session',
                target_pattern: 'src/app.ts',
              },
            },
          },
        }),
      });
    });
  });

  it('hides persistent allow grants on high and critical risk actions and rejects them in store', async () => {
    for (const risk of ['high', 'critical'] as const) {
      const tool = seedSessionWithApproval(risk, 'bash', 'rm -rf dist');
      const view = render(<ToolCard tool={tool} />);

      expect(screen.getByTestId('approval-risk-ceiling').textContent).toContain(
        `${risk} risk: persistent session/project allow disabled`,
      );
      expect(screen.queryByRole('button', { name: 'Always allow (session)' })).toBeNull();
      expect(screen.queryByRole('button', { name: 'Always allow (project)' })).toBeNull();
      expect(screen.getByRole('button', { name: 'Allow once' })).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Deny once' })).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Always deny (session)' })).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Always deny (project)' })).toBeTruthy();

      const blocked = await agentStore.handleScopedApprovalDecision('appr-1', 'allow_project', 'rm -rf dist');
      expect(blocked).toBe(false);
      expect(invoke).not.toHaveBeenCalled();
      view.unmount();
    }
  });

  it('renders FSM path, evidence trust tags, snapshot status, and scrubs leaked secrets in DecisionTraceDrawer', () => {
    render(
      <DecisionTraceDrawer
        defaultOpen
        trace={{
          request_id: 'req-1',
          session_id: SESSION_ID,
          turn_id: 'turn-1',
          call_id: 'call-1',
          trace_id: 'trace-1',
          action_id: 'edit_file',
          risk: 'medium',
          phase: 'verified',
          selected_tier: 'tier2_decision_model',
          candidate_tiers: ['tier1_deterministic', 'tier2_decision_model'],
          provider_id: 'local-laya',
          model_id: 'laya-v1',
          hop_count: 1,
          privacy_blocked_cloud: true,
          reason_code: 'calibrated_ok Authorization: Bearer super-secret-token-xyz',
          calibrated_threshold: 0.75,
          raw_confidence: 0.91,
          fsm_path: [
            'snapshotted',
            'routed',
            'inferred',
            'schema_validated',
            'policy_evaluated',
            'snapshot_rechecked',
            'executed',
            'verified',
          ],
        }}
        evidence={[
          {
            source_ref: 'src/main.ts:10-20',
            trust: 'verified_fact',
            summary: 'Buffer clean, hash matched',
          },
          {
            source_ref: 'AGENTS.md',
            trust: 'untrusted_workspace',
            summary: 'key sk-live1234567890secret should be scrubbed',
          },
          {
            source_ref: 'model:laya-v1',
            trust: 'inferred',
            summary: 'Confidence 0.91',
          },
        ]}
      />,
    );

    const fsm = screen.getByTestId('decision-trace-fsm').textContent ?? '';
    expect(fsm).toContain('snapshotted → routed → inferred');
    expect(fsm).toContain('snapshot_rechecked → executed → verified');
    expect(screen.getByText('Rechecked OK')).toBeTruthy();
    expect(screen.getByText('verified_fact')).toBeTruthy();
    expect(screen.getByText('untrusted_workspace')).toBeTruthy();
    expect(screen.getByText('inferred')).toBeTruthy();
    expect(screen.getByText(/cloud blocked \(local-only\)/i)).toBeTruthy();

    // Secret scrubbing defense-in-depth
    const drawerText = screen.getByTestId('decision-trace-drawer').textContent ?? '';
    expect(drawerText).not.toContain('super-secret-token-xyz');
    expect(drawerText).not.toContain('sk-live1234567890secret');
    expect(drawerText).toContain('[REDACTED]');
  });
});
