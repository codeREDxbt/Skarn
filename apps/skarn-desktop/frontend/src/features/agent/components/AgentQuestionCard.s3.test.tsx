import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import type { AgentQuestionPrompt } from '../../../bridge/bridge';
import { makeBootstrap } from '../../../testing/fixtures';
import { agentStore, type WorkflowToolCall } from '../agent-store';
import { AgentQuestionCard } from './AgentQuestionCard';
import { ToolCard } from './ToolCard';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const SAMPLE_PROMPT: AgentQuestionPrompt = {
  kind: 'question',
  header: 'Choose test runner',
  question: 'Which command should run verification?',
  options: [
    { id: 'opt-vitest', label: 'Vitest', description: 'Run unit tests' },
  ],
  allow_free_text: true,
};

describe('AgentQuestionCard clocked countdowns', () => {
  it('renders live countdown and triggers retroactive Pause/Resume when auto_continue_allowed is true', () => {
    const onToggle = vi.fn();
    const deadline = Date.now() + 125_000;

    const { rerender } = render(
      <AgentQuestionCard
        prompt={SAMPLE_PROMPT}
        questionTimeout={{
          deadline_ms: deadline,
          auto_continue_allowed: true,
          timed_out: false,
          paused: false,
        }}
        onToggleCountdownPaused={onToggle}
        onAnswer={vi.fn()}
      />,
    );

    const countdown = screen.getByTestId('question-countdown');
    expect(countdown.textContent).toMatch(/Auto-continue in 2:0[45]/);

    fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
    expect(onToggle).toHaveBeenCalledWith(true);

    rerender(
      <AgentQuestionCard
        prompt={SAMPLE_PROMPT}
        questionTimeout={{
          deadline_ms: deadline,
          auto_continue_allowed: true,
          timed_out: false,
          paused: true,
        }}
        onToggleCountdownPaused={onToggle}
        onAnswer={vi.fn()}
      />,
    );

    expect(screen.getByTestId('question-countdown').textContent).toMatch(/Paused · 2:0[45]/);
    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    expect(onToggle).toHaveBeenCalledWith(false);
  });

  it('renders "Unanswered, auto-continued" pill when timed_out is true', () => {
    render(
      <AgentQuestionCard
        prompt={SAMPLE_PROMPT}
        questionTimeout={{
          deadline_ms: Date.now() - 1000,
          auto_continue_allowed: true,
          timed_out: true,
          paused: false,
        }}
        onAnswer={vi.fn()}
      />,
    );

    expect(screen.getByTestId('question-timed-out-pill').textContent).toBe(
      'Unanswered, auto-continued',
    );
    expect(screen.queryByRole('button', { name: 'Send answer' })).toBeNull();
  });

  it('never renders countdown or auto-continue affordance when auto_continue_allowed is false', () => {
    render(
      <AgentQuestionCard
        prompt={SAMPLE_PROMPT}
        questionTimeout={{
          deadline_ms: Date.now() + 300_000,
          auto_continue_allowed: false,
          timed_out: false,
          paused: false,
        }}
        onToggleCountdownPaused={vi.fn()}
        onAnswer={vi.fn()}
      />,
    );

    expect(screen.queryByTestId('question-countdown')).toBeNull();
    expect(screen.queryByTestId('question-timed-out-pill')).toBeNull();
    expect(screen.queryByRole('button', { name: /Pause|Resume/i })).toBeNull();
  });

  it('dispatches SetQuestionCountdown through ToolCard and handles QuestionCountdownUpdated timeout', async () => {
    vi.mocked(invoke).mockResolvedValue({ acknowledged: true });
    agentStore.reset();
    agentStore.hydrate(
      makeBootstrap({
        workspace: { id: 'ws-q', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
        models: [{ id: 'm1', display_name: 'Model 1', available: true, tool_capability: 'supported' }],
        active_model_id: 'm1',
        chat: {
          sessions: [{ session_id: 'sess-q', title: 'Q', active: true, message_ids: [] }],
          messages: [{ message_id: 'u1', session_id: 'sess-q', role: 'user', text: 'Ask me' }],
          active_session_id: 'sess-q',
          active_request_id: 'req-q',
          active_turn_id: 'turn-q',
          pending_approval: null,
        },
      }),
    );

    act(() => {
      agentStore.handleApprovalNeeded({
        request_id: 'req-q',
        session_id: 'sess-q',
        turn_id: 'turn-q',
        approval_id: 'appr-q',
        tool_name: 'ask_user_question',
        path_or_command: 'Choose option',
        prompt: SAMPLE_PROMPT,
        question_timeout: {
          deadline_ms: Date.now() + 295_000,
          auto_continue_allowed: true,
          timed_out: false,
          paused: false,
        },
      });
    });

    const tool = agentStore.getSnapshot().messages[1]!.toolCalls![0] as WorkflowToolCall;
    const { rerender } = render(<ToolCard tool={tool} />);

    fireEvent.click(screen.getByRole('button', { name: 'Pause' }));

    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith('dispatch_command', {
        envelope: expect.objectContaining({
          session_id: null,
          payload: {
            type: 'set_question_countdown',
            payload: { request_id: 'req-q', approval_id: 'appr-q', paused: true },
          },
        }),
      });
    });

    act(() => {
      agentStore.handleQuestionCountdownUpdated({
        request_id: 'req-q',
        session_id: 'sess-q',
        turn_id: 'turn-q',
        approval_id: 'appr-q',
        timeout: {
          deadline_ms: Date.now() - 10,
          auto_continue_allowed: true,
          timed_out: true,
          paused: false,
        },
      });
    });

    const updatedTool = agentStore.getSnapshot().messages[1]!.toolCalls![0] as WorkflowToolCall;
    rerender(<ToolCard tool={updatedTool} />);
    expect(screen.getByTestId('question-timed-out-pill').textContent).toBe('Unanswered, auto-continued');
  });
});
