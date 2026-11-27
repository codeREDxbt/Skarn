import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { makeBootstrap } from '../../../testing/fixtures';
import { agentStore } from '../agent-store';
import { AgentExecutionFeed } from './AgentExecutionFeed';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const SESSION_ID = 'sess-undo-s3';

function setupTurnWithCheckpoint() {
  agentStore.reset();
  agentStore.hydrate(
    makeBootstrap({
      workspace: { id: 'ws-undo', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
      models: [{ id: 'm1', display_name: 'Model 1', available: true, tool_capability: 'supported' }],
      active_model_id: 'm1',
      chat: {
        sessions: [{ session_id: SESSION_ID, title: 'Undo S3', active: true, message_ids: [] }],
        messages: [],
        active_session_id: SESSION_ID,
        active_request_id: null,
        active_turn_id: null,
        pending_approval: null,
      },
      turn_checkpoints: [
        {
          session_id: SESSION_ID,
          turn_id: 'turn-42',
          created_at_ms: 1700000000000,
          files: [
            { path: 'src/app.ts', existed_before: true, existed_after: true },
            { path: 'src/new.ts', existed_before: false, existed_after: true },
          ],
          undone: false,
          conflicts: [],
        },
      ],
    }),
  );

  // Simulate assistant message bound to turn-42
  act(() => {
    agentStore.sendUserMessage('Edit files');
    const dispatchCall = vi.mocked(invoke).mock.calls.find(([cmd]) => cmd === 'dispatch_command');
    const reqId = (
      dispatchCall?.[1] as { envelope?: { payload?: { payload?: { request_id?: string } } } }
    )?.envelope?.payload?.payload?.request_id ?? 'req-fallback';
    agentStore.handleChatDelta({ request_id: reqId, session_id: SESSION_ID, text: 'Done.' });
    agentStore.handleAgentTurnDone({
      request_id: reqId,
      session_id: SESSION_ID,
      turn_id: 'turn-42',
      total_tokens: 100,
      cost_usd: 0.01,
      num_tools: 2,
      approved: 2,
      denied: 0,
      completed: true,
      end_message: 'Done.',
    });
  });
}

describe('AgentExecutionFeed Turn Undo/Reapply & CheckpointConflict', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue({ acknowledged: true });
  });

  it('dispatches UndoAgentTurn and flips to Reapply when TurnCheckpointUpdated arrives', async () => {
    setupTurnWithCheckpoint();
    const snap = agentStore.getSnapshot();
    render(
      <AgentExecutionFeed
        messages={snap.messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={snap.metrics}
      />,
    );

    const undoBtn = screen.getByRole('button', { name: 'Undo turn' });
    fireEvent.click(undoBtn);

    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('dispatch_command', {
        envelope: expect.objectContaining({
          session_id: null,
          payload: {
            type: 'undo_agent_turn',
            payload: { session_id: SESSION_ID, turn_id: 'turn-42' },
          },
        }),
      });
      expect(agentStore.getSnapshot().checkpointBusyTurnId).toBeNull();
    });

    act(() => {
      agentStore.handleTurnCheckpointUpdated({
        session_id: SESSION_ID,
        turn_id: 'turn-42',
        created_at_ms: 1700000000000,
        files: [
          { path: 'src/app.ts', existed_before: true, existed_after: true },
          { path: 'src/new.ts', existed_before: false, existed_after: true },
        ],
        undone: true,
        conflicts: [],
      });
    });

    const reapplyBtn = await screen.findByRole('button', { name: 'Reapply' });
    fireEvent.click(reapplyBtn);

    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('dispatch_command', {
        envelope: expect.objectContaining({
          session_id: null,
          payload: {
            type: 'reapply_agent_turn',
            payload: { session_id: SESSION_ID, turn_id: 'turn-42' },
          },
        }),
      });
    });
  });

  it('renders an explicit conflict dialog listing drifted file paths on CheckpointConflict abort', async () => {
    setupTurnWithCheckpoint();
    vi.mocked(invoke).mockImplementation(async (_cmd, args) => {
      const env = (args as { envelope?: { payload?: { type?: string } } })?.envelope;
      if (env?.payload?.type === 'undo_agent_turn') {
        agentStore.handleTurnCheckpointUpdated({
          session_id: SESSION_ID,
          turn_id: 'turn-42',
          created_at_ms: 1700000000000,
          files: [{ path: 'src/app.ts', existed_before: true, existed_after: true }],
          undone: false,
          conflicts: [
            { path: 'src/app.ts', reason: 'File modified manually after turn checkpoint' },
          ],
        });
        throw { code: 'checkpoint_conflict', message: 'Manual edits detected on src/app.ts', retryable: false };
      }
      return { acknowledged: true };
    });

    const snap = agentStore.getSnapshot();
    render(
      <AgentExecutionFeed
        messages={snap.messages}
        inFlight={false}
        thinkingState="breathing"
        metrics={snap.metrics}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Undo turn' }));

    const dialog = await screen.findByTestId('checkpoint-conflict-dialog');
    expect(within(dialog).getByText(/Undo turn aborted/i)).toBeTruthy();
    expect(within(dialog).getByText('src/app.ts')).toBeTruthy();
    expect(within(dialog).getByText('File modified manually after turn checkpoint')).toBeTruthy();

    fireEvent.click(within(dialog).getByRole('button', { name: 'Dismiss' }));
    expect(screen.queryByTestId('checkpoint-conflict-dialog')).toBeNull();
  });
});
