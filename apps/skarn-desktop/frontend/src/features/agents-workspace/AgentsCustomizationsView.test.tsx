import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBootstrap } from '../../testing/fixtures';

const harness = vi.hoisted(() => {
  const initial = () => ({
    selectedModel: 'gpt-test',
    availableModels: [
      { id: 'gpt-test', display_name: 'GPT Test', available: true },
      { id: 'deepseek-chat', display_name: 'DeepSeek Chat', available: true },
    ],
    reasoningEffort: 'standard',
    modelSettings: { systemInstructions: '', temperature: 1, customModelId: '' },
  });
  return {
    initial,
    state: initial(),
    listeners: new Set<() => void>(),
    updateModelSettings: vi.fn(),
  };
});

vi.mock('../agent/agent-store', () => ({
  agentStore: {
    getSnapshot: () => harness.state,
    subscribe: (listener: () => void) => {
      harness.listeners.add(listener);
      return () => harness.listeners.delete(listener);
    },
    updateModelSettings: harness.updateModelSettings,
  },
}));

import { AgentsCustomizationsView } from './AgentsCustomizationsView';

describe('AgentsCustomizationsView', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    harness.state = harness.initial();
  });

  it('owns only global instructions and omits duplicate provider, model, and effort controls', async () => {
    harness.updateModelSettings.mockImplementation(async (patch: { systemInstructions?: string }) => {
      harness.state = {
        ...harness.state,
        modelSettings: { ...harness.state.modelSettings, ...patch },
      };
      harness.listeners.forEach((listener) => listener());
    });
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    expect(screen.queryByLabelText('Model')).toBeNull();
    expect(screen.queryByLabelText('Reasoning effort')).toBeNull();
    expect(screen.queryByText('Assistant providers')).toBeNull();

    fireEvent.change(screen.getByLabelText('Instructions'), {
      target: { value: 'Keep changes small.' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save instructions' }));
    await waitFor(() =>
      expect(harness.updateModelSettings).toHaveBeenCalledWith({
        systemInstructions: 'Keep changes small.',
      }),
    );
    await waitFor(() =>
      expect(screen.getByText('Instructions saved for subsequent requests.')).toBeTruthy(),
    );
    expect(screen.getByText(/not an ongoing turn/)).toBeTruthy();
  });

  it('reflects shared instructions updates from another surface', () => {
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    act(() => {
      harness.state = {
        ...harness.state,
        modelSettings: {
          ...harness.state.modelSettings,
          systemInstructions: 'Changed elsewhere.',
        },
      };
      harness.listeners.forEach((listener) => listener());
    });
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe(
      'Changed elsewhere.',
    );
  });

  it('reports thrown preference persistence failures without success feedback', async () => {
    harness.updateModelSettings.mockImplementation(() => {
      throw new Error('Storage unavailable');
    });
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.change(screen.getByLabelText('Instructions'), {
      target: { value: 'New instructions' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save instructions' }));
    await waitFor(() =>
      expect(screen.getByRole('alert').textContent).toContain('Storage unavailable'),
    );
    expect(screen.queryByText('Instructions saved for subsequent requests.')).toBeNull();
  });

  it('preserves an edited draft when another window updates shared instructions', () => {
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.change(screen.getByLabelText('Instructions'), {
      target: { value: 'My unsaved draft.' },
    });
    act(() => {
      harness.state = {
        ...harness.state,
        modelSettings: {
          ...harness.state.modelSettings,
          systemInstructions: 'Saved in another window.',
        },
      };
      harness.listeners.forEach((listener) => listener());
    });
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe(
      'My unsaved draft.',
    );
    expect(screen.getByText(/Your unsaved draft is preserved/)).toBeTruthy();
    expect(harness.updateModelSettings).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Use shared instructions' }));
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe(
      'Saved in another window.',
    );
    expect(screen.queryByText(/Your unsaved draft is preserved/)).toBeNull();
  });

  it('surfaces rejected native preference writes without success feedback', async () => {
    harness.updateModelSettings.mockRejectedValue(new Error('Native preferences write failed'));
    render(<AgentsCustomizationsView bootstrap={makeBootstrap()} />);
    fireEvent.change(screen.getByLabelText('Instructions'), {
      target: { value: 'My draft.' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save instructions' }));
    await waitFor(() =>
      expect(screen.getByRole('alert').textContent).toContain('Native preferences write failed'),
    );
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toBe('My draft.');
    expect(screen.queryByText('Instructions saved for subsequent requests.')).toBeNull();
  });
});
