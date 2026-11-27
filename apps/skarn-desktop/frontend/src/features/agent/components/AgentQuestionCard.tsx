// Selora IDE — Phase 9a structured question card (ROADMAP §"Phase 9", task 9a).
//
// A question parks the same turn an approval does; it is not a second waiting
// mechanism. The card renders the backend's `AgentQuestionPrompt` verbatim and
// routes the answer through `agentStore.answerQuestion` (the existing
// AgentDecision command plus the additive `resolution` payload).
import React, { useEffect, useId, useState } from 'react';
import { Check, Clock, DirectSend } from '../../../components/icons';
import type {
  AgentDecisionResolution,
  AgentQuestionPrompt,
  QuestionTimeoutDto,
} from '../../../bridge/bridge';

export type AgentQuestionCardProps = {
  prompt: AgentQuestionPrompt;
  submitting?: boolean;
  error?: string | null;
  resolution?: AgentDecisionResolution;
  questionTimeout?: QuestionTimeoutDto;
  onToggleCountdownPaused?: (paused: boolean) => void;
  onAnswer: (optionId: string | null, freeText: string) => void;
};

function formatCountdown(remainingSeconds: number): string {
  const mins = Math.floor(remainingSeconds / 60);
  const secs = remainingSeconds % 60;
  return `${mins}:${secs.toString().padStart(2, '0')}`;
}

function answeredLabel(
  prompt: AgentQuestionPrompt,
  resolution: AgentDecisionResolution | undefined,
): string | null {
  if (!resolution || resolution.kind !== 'question') return null;
  const option = resolution.option_id
    ? prompt.options.find((candidate) => candidate.id === resolution.option_id)
    : undefined;
  if (option && resolution.text) return `${option.label} — ${resolution.text}`;
  if (option) return option.label;
  return resolution.text ?? null;
}

export const AgentQuestionCard: React.FC<AgentQuestionCardProps> = ({
  prompt,
  submitting = false,
  error,
  resolution,
  questionTimeout,
  onToggleCountdownPaused,
  onAnswer,
}) => {
  const groupName = useId();
  const [selected, setSelected] = useState<string | null>(null);
  const [freeText, setFreeText] = useState('');
  const [nowMs, setNowMs] = useState(() => Date.now());
  const answered = answeredLabel(prompt, resolution);
  const autoContinueAllowed = questionTimeout?.auto_continue_allowed === true;
  const timedOut = autoContinueAllowed && questionTimeout?.timed_out === true;
  const paused = Boolean(questionTimeout?.paused);

  useEffect(() => {
    if (!autoContinueAllowed || timedOut || paused || answered) return;
    setNowMs(Date.now());
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
  }, [autoContinueAllowed, timedOut, paused, answered, questionTimeout?.deadline_ms]);

  const remainingSeconds =
    autoContinueAllowed && questionTimeout
      ? Math.max(0, Math.ceil((questionTimeout.deadline_ms - nowMs) / 1000))
      : 0;
  const freeTextAllowed = prompt.allow_free_text === true || prompt.options.length === 0;
  const canSend = !submitting && !timedOut && (selected !== null || freeText.trim() !== '');

  return (
    <section
      className="agent-decision-card agent-question-card"
      aria-label="Agent question"
    >
      <header className="agent-decision-header">
        <span className="agent-decision-kind">Question</span>
        {prompt.header ? (
          <span className="agent-decision-context">{prompt.header}</span>
        ) : null}
        {autoContinueAllowed && !answered && (
          timedOut ? (
            <span
              className="agent-question-timeout-pill is-timed-out"
              role="status"
              data-testid="question-timed-out-pill"
            >
              Unanswered, auto-continued
            </span>
          ) : (
            <span className="agent-question-countdown" data-testid="question-countdown">
              <Clock size={11} aria-hidden="true" />
              <span>
                {paused
                  ? `Paused · ${formatCountdown(remainingSeconds)}`
                  : `Auto-continue in ${formatCountdown(remainingSeconds)}`}
              </span>
              {onToggleCountdownPaused && (
                <button
                  type="button"
                  className="agent-question-pause-btn"
                  disabled={submitting}
                  onClick={() => onToggleCountdownPaused(!paused)}
                >
                  {paused ? 'Resume' : 'Pause'}
                </button>
              )}
            </span>
          )
        )}
      </header>

      <p className="agent-decision-question">{prompt.question}</p>

      {answered ? (
        <div className="agent-decision-answered" role="status">
          <Check size={12} aria-hidden="true" />
          <span>{answered}</span>
        </div>
      ) : timedOut ? (
        <div className="agent-decision-answered is-timed-out" role="status">
          <span>Unanswered, auto-continued</span>
        </div>
      ) : (
        <>
          {prompt.options.length > 0 ? (
            <fieldset className="agent-question-options">
              <legend className="agent-question-legend">Choose an answer</legend>
              {prompt.options.map((option, index) => (
                <label key={option.id} className="agent-question-option">
                  <input
                    type="radio"
                    className="agent-question-option-input"
                    name={groupName}
                    value={option.id}
                    checked={selected === option.id}
                    disabled={submitting}
                    onChange={() => setSelected(option.id)}
                  />
                  <span className="agent-question-option-body">
                    <span className="agent-question-option-label">
                      {option.label}
                      {index === 0 ? <span className="visually-hidden"> (first option)</span> : null}
                    </span>
                    {option.description ? (
                      <span className="agent-question-option-desc">
                        {option.description}
                      </span>
                    ) : null}
                  </span>
                </label>
              ))}
            </fieldset>
          ) : null}

          {freeTextAllowed ? (
            <label className="agent-question-free">
              <span className="agent-question-free-label">
                {prompt.options.length > 0 ? 'Or add detail (optional)' : 'Your answer'}
              </span>
              <textarea
                className="agent-question-textarea"
                value={freeText}
                rows={2}
                disabled={submitting}
                placeholder={prompt.free_text_placeholder ?? 'Type an answer…'}
                onChange={(event) => setFreeText(event.target.value)}
              />
            </label>
          ) : null}

          {error ? (
            <p className="agent-decision-error" role="alert">
              {error}
            </p>
          ) : null}

          <div className="agent-decision-actions">
            <button
              type="button"
              className="agent-decision-primary"
              disabled={!canSend}
              aria-busy={submitting}
              onClick={() => onAnswer(selected, freeText)}
            >
              <DirectSend size={12} aria-hidden="true" />
              <span>{submitting ? 'Sending…' : 'Send answer'}</span>
            </button>
          </div>
        </>
      )}
    </section>
  );
};