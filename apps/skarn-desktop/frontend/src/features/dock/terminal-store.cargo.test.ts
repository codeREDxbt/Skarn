// Cargo build completion is reported from raw terminal output.
//
// A `Finished` line is a success toast; `error: could not compile` is a
// failure toast. The split case proves the detector keeps the partial line
// between output chunks (the shell can split anywhere).
import { beforeEach, describe, expect, it, vi } from 'vitest';

const bridgeMocks = vi.hoisted(() => ({
  createTerminal: vi.fn(),
  terminalInput: vi.fn(),
  terminalResize: vi.fn(),
  terminalClose: vi.fn(),
}));

const toastMocks = vi.hoisted(() => ({ pushLocal: vi.fn() }));

vi.mock('../../bridge/bridge', () => bridgeMocks);
vi.mock('../toasts/toast-store', () => ({ toastStore: toastMocks }));

import { terminalStore } from './terminal-store';

const created = {
  terminal_id: 'term-1',
  shell: '/usr/bin/bash',
  cwd: '/workspace',
};

beforeEach(() => {
  terminalStore.reset();
  vi.clearAllMocks();
});

describe('cargo build notifications', () => {
  it('toasts a success for a Finished line split across output events', () => {
    terminalStore.handleCreated(created);
    terminalStore.handleOutput({ terminal_id: 'term-1', data: '    Finished `dev` pro' });
    terminalStore.handleOutput({
      terminal_id: 'term-1',
      data: 'file [unoptimized] target(s) in 1.23s\r\n',
    });

    expect(toastMocks.pushLocal).toHaveBeenCalledTimes(1);
    expect(toastMocks.pushLocal).toHaveBeenCalledWith(
      'success',
      'Cargo build finished: Finished `dev` profile [unoptimized] target(s) in 1.23s',
    );
  });

  it('toasts a failure when cargo cannot compile the crate', () => {
    terminalStore.handleCreated(created);
    terminalStore.handleOutput({
      terminal_id: 'term-1',
      data: 'error: could not compile `selora-app` (bin "selora") due to 2 previous errors\n',
    });

    expect(toastMocks.pushLocal).toHaveBeenCalledTimes(1);
    expect(toastMocks.pushLocal).toHaveBeenCalledWith(
      'error',
      'Cargo build failed: error: could not compile `selora-app` (bin "selora") due to 2 previous errors',
    );
  });

  it('ignores unrelated compiler output and unknown terminals', () => {
    terminalStore.handleCreated(created);
    terminalStore.handleOutput({ terminal_id: 'term-1', data: 'Compiling selora-app v0.1.0\n' });
    terminalStore.handleOutput({ terminal_id: 'missing', data: 'error: could not compile x\n' });

    expect(toastMocks.pushLocal).not.toHaveBeenCalled();
  });

  it('does not toast the same completion twice after the line is drained', () => {
    terminalStore.handleCreated(created);
    terminalStore.handleOutput({
      terminal_id: 'term-1',
      data: 'Finished `release` profile [optimized] target(s) in 4.00s\n',
    });
    terminalStore.handleOutput({ terminal_id: 'term-1', data: 'running 12 tests\n' });

    expect(toastMocks.pushLocal).toHaveBeenCalledTimes(1);
  });
});