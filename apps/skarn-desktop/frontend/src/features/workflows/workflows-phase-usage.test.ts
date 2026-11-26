// Per-phase usage must survive the terminal `workflow_finished` event.
//
// The snapshot path (`hydratePersisted`) and the live path (`applyEvent`) both
// read the same additive rows. A finish without usage keeps the prior snapshot:
// the event is additive, never a truncation.
import { beforeEach, describe, expect, it } from 'vitest';
import { workflowsStore } from './workflows-store';

const STARTED = {
  kind: 'started' as const,
  run_id: 'run_1',
  request_id: 'req_1',
  name: 'Usage review',
  max_cost_usd: 5,
  phases: ['scan', 'verify'],
};

beforeEach(() => {
  workflowsStore.reset();
});

describe('per-phase usage on the finished event', () => {
  it('stores the terminal rows and keeps the receipt totals', () => {
    workflowsStore.applyEvent(STARTED);
    workflowsStore.applyEvent({
      kind: 'finished',
      run_id: 'run_1',
      status: 'completed',
      tokens: 2000,
      cost_usd: 0.2,
      phase_usage: [
        { phase: 'scan', tokens: 1200, cost_usd: 0.12 },
        { phase: 'verify', tokens: 0, cost_usd: 0 },
      ],
    });

    const run = workflowsStore.getSnapshot().runs[0];
    expect(run?.status).toBe('completed');
    expect(run?.tokens).toBe(2000);
    expect(run?.cost_usd).toBe(0.2);
    expect(run?.phase_usage).toEqual([
      { phase: 'scan', tokens: 1200, cost_usd: 0.12 },
      { phase: 'verify', tokens: 0, cost_usd: 0 },
    ]);
  });

  it('keeps the prior snapshot when an older terminal event omits usage', () => {
    workflowsStore.hydratePersisted([
      {
        run_id: 'run_1',
        name: 'Usage review',
        status: 'running',
        resumable: false,
        phases: ['scan', 'verify'],
        phase_usage: [
          { phase: 'scan', tokens: 1200, cost_usd: 0.12 },
          { phase: 'verify', tokens: 0, cost_usd: 0 },
        ],
        tokens: 1200,
        cost_usd: 0.12,
      },
    ]);
    workflowsStore.applyEvent({
      kind: 'finished',
      run_id: 'run_1',
      status: 'completed',
      tokens: 1200,
      cost_usd: 0.12,
    });

    const run = workflowsStore.getSnapshot().runs[0];
    expect(run?.phase_usage).toEqual([
      { phase: 'scan', tokens: 1200, cost_usd: 0.12 },
      { phase: 'verify', tokens: 0, cost_usd: 0 },
    ]);
  });

  it('normalises an unknown or negative row to explicit zero facts', () => {
    workflowsStore.applyEvent(STARTED);
    workflowsStore.applyEvent({
      kind: 'finished',
      run_id: 'run_1',
      status: 'completed',
      phase_usage: [
        { phase: ' scan ', tokens: 1200, cost_usd: 0.12 },
        { phase: 'verify', tokens: -3, cost_usd: Number.NaN },
      ],
    });

    expect(workflowsStore.getSnapshot().runs[0]?.phase_usage).toEqual([
      { phase: 'scan', tokens: 1200, cost_usd: 0.12 },
      { phase: 'verify', tokens: 0, cost_usd: 0 },
    ]);
  });
});