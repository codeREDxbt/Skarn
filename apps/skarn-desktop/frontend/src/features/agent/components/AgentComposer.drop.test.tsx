import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { makeBootstrap } from '../../../testing/fixtures';
import { agentStore } from '../agent-store';
import { AgentComposer } from './AgentComposer';

describe('workspace file drops', () => {
  beforeEach(() => {
    agentStore.reset();
    agentStore.hydrate(makeBootstrap({
      workspace: { id: 'ws', core_instance_id: 'core-1', root: 'C:/repo', display_name: 'repo' },
    }));
  });

  it('attaches only absolute files inside the workspace and deduplicates them', () => {
    const { container } = render(
      <AgentComposer inFlight={false} streamingActivityLevel={0} permissionMode="auto_edit" reasoningEffort="standard" promptHistory={[]} />,
    );
    const composer = container.querySelector('.agent-composer')!;
    fireEvent.drop(composer, { dataTransfer: { files: [
      { path: 'C:/repo/src/main.ts' }, { path: 'C:/repo/src/main.ts' },
      { path: 'C:/outside/secret' }, { path: '../secret' }, { path: 'src/unknown.ts' },
    ] } });
    expect(screen.getAllByRole('button', { name: 'Remove attachment main.ts' })).toHaveLength(1);
    expect(screen.queryByText('secret')).toBeNull();
    expect(screen.queryByText('unknown.ts')).toBeNull();
  });
});
