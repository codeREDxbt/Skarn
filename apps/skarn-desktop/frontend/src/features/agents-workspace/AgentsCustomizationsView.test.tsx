import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => {
  const initial = () => ({
    selectedModel: 'gpt-test',
    availableModels: [
      { id: 'gpt-test', display_name: 'GPT Test', available: true },
      { id: 'deepseek-chat', display_name: 'DeepSeek Chat', available: true },
      { id: 'unavailable', display_name: 'Unavailable', available: false },
    ],
    reasoningEffort: 'standard',
    modelSettings: { systemInstructions: '', temperature: 1, customModelId: '' },
  });
  return { initial, state: initial(), listeners: new Set<() => void>(), updateSettings: vi.fn(), setSelectedModel: vi.fn(), setReasoningEffort: vi.fn(), updateModelSettings: vi.fn() };
});

vi.mock('../../bridge/bridge', () => ({ updateSettings: harness.updateSettings }));
vi.mock('../agent/agent-store', () => ({
  agentStore: {
    getSnapshot: () => harness.state,
    subscribe: (listener: () => void) => { harness.listeners.add(listener); return () => harness.listeners.delete(listener); },
    setSelectedModel: harness.setSelectedModel,
    setReasoningEffort: harness.setReasoningEffort,
    updateModelSettings: harness.updateModelSettings,
  },
}));
vi.mock('../auth/ConnectModal', () => ({ PROVIDER_PRESETS: [
  { id: 'openai', name: 'OpenAI Direct' },
  { id: 'deepseek', name: 'DeepSeek' },
] }));

import { AgentsCustomizationsView } from './AgentsCustomizationsView';

describe('AgentsCustomizationsView', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    harness.state = harness.initial();
    harness.updateSettings.mockResolvedValue(undefined);
  });

  it('uses shared setters for subsequent-request preferences without backend settings writes', async () => {
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.change(screen.getByLabelText('Model'), { target: { value: 'deepseek-chat' } });
    await waitFor(() => expect(harness.setSelectedModel).toHaveBeenCalledWith('deepseek-chat'));
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('subsequent requests'));
    fireEvent.change(screen.getByLabelText('Reasoning effort'), { target: { value: 'max' } });
    await waitFor(() => expect(harness.setReasoningEffort).toHaveBeenCalledWith('max'));
    await waitFor(() => expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).disabled).toBe(false));
    fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'Keep changes small.' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save instructions' }));
    await waitFor(() => expect(harness.updateModelSettings).toHaveBeenCalledWith({ systemInstructions: 'Keep changes small.' }));
    expect(harness.updateSettings).not.toHaveBeenCalled();
    expect(screen.getByText(/not an ongoing turn/)).toBeTruthy();
  });

  it('reflects shared preference changes from another surface', () => {
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    act(() => {
      harness.state = { ...harness.state, selectedModel: 'deepseek-chat', reasoningEffort: 'quick', modelSettings: { ...harness.state.modelSettings, systemInstructions: 'Changed elsewhere.' } };
      harness.listeners.forEach((listener) => listener());
    });
    expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe('deepseek-chat');
    expect((screen.getByLabelText('Reasoning effort') as HTMLSelectElement).value).toBe('quick');
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe('Changed elsewhere.');
  });

  it('reports thrown preference persistence failures without success feedback', async () => {
    harness.updateModelSettings.mockImplementation(() => { throw new Error('Storage unavailable'); });
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'New instructions' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save instructions' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Storage unavailable'));
    expect(screen.queryByText('Preference updated for subsequent requests.')).toBeNull();
  });

  it('preserves an edited draft when another window updates shared instructions', () => {
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'My unsaved draft.' } });
    act(() => {
      harness.state = { ...harness.state, modelSettings: { ...harness.state.modelSettings, systemInstructions: 'Saved in another window.' } };
      harness.listeners.forEach((listener) => listener());
    });
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe('My unsaved draft.');
    expect(screen.getByText(/Your unsaved draft is preserved/)).toBeTruthy();
    expect(harness.updateModelSettings).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Use shared instructions' }));
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe('Saved in another window.');
    expect(screen.queryByText(/Your unsaved draft is preserved/)).toBeNull();
  });

  it('surfaces rejected native preference writes without success feedback', async () => {
    harness.updateModelSettings.mockRejectedValue(new Error('Native preferences write failed'));
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.change(screen.getByLabelText('Instructions'), { target: { value: 'My draft.' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save instructions' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Native preferences write failed'));
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe('My draft.');
    expect(screen.queryByText('Preference updated for subsequent requests.')).toBeNull();
  });

  it('writes provider settings without a session and waits for the authoritative snapshot', async () => {
    const bootstrap = makeBootstrap();
    const { rerender } = render(<AgentsCustomizationsView bootstrap={bootstrap} />);
    fireEvent.click(screen.getByLabelText('Enable DeepSeek'));
    await waitFor(() => expect(harness.updateSettings).toHaveBeenCalledWith(
      { disabled_providers: ['deepseek'] },
      { core_instance_id: bootstrap.core_instance_id, workspace_id: bootstrap.workspace.id, session_id: null },
    ));
    expect(screen.getByRole('status').textContent).toContain('subsequent requests');
    expect((screen.getByLabelText('Enable DeepSeek') as HTMLInputElement).checked).toBe(true);
    rerender(<AgentsCustomizationsView bootstrap={{ ...bootstrap, settings: { ...bootstrap.settings, disabled_providers: ['deepseek'] } }} />);
    expect((screen.getByLabelText('Enable DeepSeek') as HTMLInputElement).checked).toBe(false);
    expect(screen.queryByRole('option', { name: 'DeepSeek Chat' })).toBeNull();
    expect(harness.setSelectedModel).not.toHaveBeenCalled();
  });

  it('reports backend provider failures and leaves the authoritative value unchanged', async () => {
    harness.updateSettings.mockRejectedValue(new Error('Backend offline'));
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.click(screen.getByLabelText('Enable DeepSeek'));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Backend offline'));
    expect((screen.getByLabelText('Enable DeepSeek') as HTMLInputElement).checked).toBe(true);
  });

  it('preserves a selected disabled-provider model and exposes custom override precedence', async () => {
    harness.state = { ...harness.state, selectedModel: 'deepseek-chat', modelSettings: { ...harness.state.modelSettings, customModelId: 'explicit-model' } };
    const bootstrap = makeBootstrap();
    const onOpenConnect = vi.fn();
    render(<AgentsCustomizationsView bootstrap={{ ...bootstrap, active_model_id: 'deepseek-chat', settings: { ...bootstrap.settings, disabled_providers: ['deepseek'] } }} onOpenConnect={onOpenConnect} />);
    expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe('deepseek-chat');
    expect(screen.getByText(/current model is unchanged/)).toBeTruthy();
    expect(screen.getByText('explicit-model')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Use selected model instead' }));
    await waitFor(() => expect(harness.updateModelSettings).toHaveBeenCalledWith({ customModelId: '' }));
    fireEvent.click(screen.getByRole('button', { name: 'Manage provider connection' }));
    expect(onOpenConnect).toHaveBeenCalledOnce();
  });
});
