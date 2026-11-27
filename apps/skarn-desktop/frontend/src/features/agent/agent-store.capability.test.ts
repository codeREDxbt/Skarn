import { createElement } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import type { Model } from '../../bridge/bridge';
import { makeBootstrap } from '../../testing/fixtures';
import { agentStore, getComposerGate, resolveModelId } from './agent-store';
import { AgentComposer } from './components/AgentComposer';

const WORKSPACE = {
  id: 'ws-cap',
  core_instance_id: 'core-cap',
  root: 'C:/repo',
  display_name: 'repo',
};

function makeModel(overrides: Partial<Model> & Pick<Model, 'id'>): Model {
  return {
    display_name: overrides.id,
    available: true,
    ...overrides,
  };
}

describe('agent-store capability gating & model preservation', () => {
  beforeEach(() => {
    agentStore.reset();
    localStorage.clear();
  });

  it('blocks unsupported and retired models locally while keeping supported and unverified models sendable', () => {
    const models: Model[] = [
      makeModel({
        id: 'mistralai/mistral-large-4-1t-a52b',
        tool_capability: 'supported',
        available: true,
      }),
      makeModel({
        id: 'chat/only-model',
        tool_capability: 'unsupported',
        available: true,
      }),
      makeModel({
        id: 'unverified/nvidia-model',
        tool_capability: 'unknown',
        available: true,
      }),
      makeModel({
        id: 'meta/muse-glimmer-30b',
        tool_capability: 'unsupported',
        available: false,
      }),
    ];

    agentStore.hydrate(
      makeBootstrap({
        workspace: WORKSPACE,
        connection: 'connected',
        models,
        active_model_id: 'mistralai/mistral-large-4-1t-a52b',
      }),
    );

    expect(getComposerGate(agentStore.getSnapshot())).toEqual({
      ready: true,
      reason: null,
      message: null,
    });
    expect(resolveModelId(agentStore.getSnapshot())).toBe('mistralai/mistral-large-4-1t-a52b');

    // Unsupported model: blocked locally for agent dispatch.
    agentStore.setSelectedModel('chat/only-model');
    expect(getComposerGate(agentStore.getSnapshot())).toMatchObject({
      ready: false,
      reason: 'unsupported_tools',
    });

    // Unverified (unknown) model: stays unblocked in UI so backend runs cached probe.
    agentStore.setSelectedModel('unverified/nvidia-model');
    expect(getComposerGate(agentStore.getSnapshot())).toEqual({
      ready: true,
      reason: null,
      message: null,
    });

    // Retired/EOL model: blocked locally as unavailable.
    agentStore.setSelectedModel('meta/muse-glimmer-30b');
    expect(getComposerGate(agentStore.getSnapshot())).toMatchObject({
      ready: false,
      reason: 'unavailable_model',
    });
  });

  it('preserves disappeared selectedModel across catalog refresh instead of switching to catalog[0]', () => {
    const initialModels: Model[] = [
      makeModel({ id: 'first/model', tool_capability: 'supported' }),
      makeModel({ id: 'explicit/selected-model', tool_capability: 'supported' }),
    ];
    agentStore.hydrate(
      makeBootstrap({
        workspace: WORKSPACE,
        connection: 'connected',
        models: initialModels,
        active_model_id: 'explicit/selected-model',
      }),
    );

    const refreshedWithoutSelected: Model[] = [
      makeModel({ id: 'first/model', tool_capability: 'supported' }),
      makeModel({ id: 'another/model', tool_capability: 'unknown' }),
    ];
    agentStore.setAvailableModels(refreshedWithoutSelected, 'explicit/selected-model');

    expect(agentStore.getSnapshot().selectedModel).toBe('explicit/selected-model');
    expect(resolveModelId(agentStore.getSnapshot())).toBeNull();
    expect(getComposerGate(agentStore.getSnapshot())).toMatchObject({
      ready: false,
      reason: 'missing_model',
    });
  });

  it('disables tool-unsupported and retired models and excludes unrecommended retired models in AgentComposer', () => {
    const models: Model[] = [
      makeModel({
        id: 'mistralai/mistral-large-4-1t-a52b',
        display_name: 'Mistral Large',
        tool_capability: 'supported',
        available: true,
      }),
      makeModel({
        id: 'chat/only-model',
        display_name: 'Chat Only Model',
        tool_capability: 'unsupported',
        available: true,
      }),
      makeModel({
        id: 'meta/muse-glimmer-30b',
        display_name: 'Muse Glimmer 30B',
        tool_capability: 'unsupported',
        available: false,
      }),
    ];

    agentStore.hydrate(
      makeBootstrap({
        workspace: WORKSPACE,
        connection: 'connected',
        models,
        active_model_id: 'mistralai/mistral-large-4-1t-a52b',
      }),
    );

    render(
      createElement(AgentComposer, {
        inFlight: false,
        streamingActivityLevel: 0,
        permissionMode: 'auto_edit',
        reasoningEffort: 'standard',
        promptHistory: [],
      }),
    );

    fireEvent.click(screen.getByTitle('Active Model: click to switch or search'));

    // Retired non-selected model is omitted from active recommendations.
    expect(screen.queryByText('Muse Glimmer 30B')).toBeNull();

    // Supported model is selectable.
    const mistralBtn = screen.getByRole('menuitem', { name: /Mistral Large/i }) as HTMLButtonElement;
    expect(mistralBtn.disabled).toBe(false);

    // Unsupported model is visibly labeled and disabled in the agent model picker.
    const chatOnlyBtn = screen.getByRole('menuitem', { name: /Chat Only Model/i }) as HTMLButtonElement;
    expect(chatOnlyBtn.disabled).toBe(true);
    expect(screen.getByText('No tools')).toBeTruthy();
  });
});
