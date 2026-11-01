import { useEffect, useId, useState, useSyncExternalStore } from 'react';
import { updateSettings, type BootstrapState } from '../../bridge/bridge';
import { agentStore } from '../agent/agent-store';
import type { ReasoningEffort } from '../agent/types';
import { PROVIDER_PRESETS } from '../auth/ConnectModal';
import { activeProviderId, isProviderDisabled, providerIdForModel, withProviderEnabled } from '../auth/provider-visibility';
import './agents-customizations.css';

export type AgentsCustomizationsViewProps = {
  bootstrap: BootstrapState;
  onOpenConnect?: () => void;
};

export function AgentsCustomizationsView({ bootstrap, onOpenConnect }: AgentsCustomizationsViewProps) {
  const agent = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const id = useId();
  const sharedInstructions = agent.modelSettings.systemInstructions;
  const [instructionsDraft, setInstructionsDraft] = useState({ value: sharedInstructions, base: sharedInstructions });
  const instructions = instructionsDraft.value;
  const instructionsConflict = instructionsDraft.base !== sharedInstructions && instructions !== sharedInstructions;
  const [feedback, setFeedback] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const disabledProviders = bootstrap.settings.disabled_providers ?? [];
  const activeProvider = activeProviderId(bootstrap.active_model_id, bootstrap.models);
  const models = agent.availableModels.filter((model) => {
    const provider = providerIdForModel(model.id, model.display_name);
    return model.id === agent.selectedModel || provider === null || !isProviderDisabled(disabledProviders, provider);
  });

  useEffect(() => {
    setInstructionsDraft((draft) => (
      draft.value === draft.base || draft.value === sharedInstructions
        ? { value: sharedInstructions, base: sharedInstructions }
        : draft
    ));
  }, [sharedInstructions]);

  // Await shared native preference writes and surface propagated persistence errors.
  const changePreference = async (action: () => unknown) => {
    setFeedback(null);
    setError(null);
    setBusy(true);
    try {
      await action();
      setFeedback('Preference updated for subsequent requests.');
    } catch (cause) {
      setError(`Preference update failed: ${cause instanceof Error ? cause.message : String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  const toggleProvider = async (provider: string, enabled: boolean) => {
    setFeedback(null);
    setError(null);
    setBusy(true);
    try {
      await updateSettings({ disabled_providers: withProviderEnabled(disabledProviders, provider, enabled) }, {
        core_instance_id: bootstrap.core_instance_id,
        workspace_id: bootstrap.workspace.id,
        session_id: null,
      });
      setFeedback('Provider preference saved for subsequent requests. Controls reflect the backend settings snapshot.');
    } catch (cause) {
      setError(`Provider settings update failed: ${cause instanceof Error ? cause.message : String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="agents-customizations" aria-labelledby={`${id}-title`}>
      <header>
        <h1 id={`${id}-title`}>Customizations</h1>
        <p>Shape how your agent works. Model, reasoning, and instructions apply to subsequent requests, not an ongoing turn.</p>
      </header>
      {error ? <p className="agents-customizations-error" role="alert">{error}</p> : null}
      {feedback ? <p role="status">{feedback}</p> : null}
      <fieldset className="agents-customizations-card" disabled={busy}>
        <legend>Agent preferences</legend>
        <label htmlFor={`${id}-model`}>Model</label>
        <select id={`${id}-model`} value={agent.selectedModel} onChange={(event) => {
          const value = event.target.value;
          void changePreference(() => agentStore.setSelectedModel(value));
        }}>
          <option value="" disabled>Select a connected model</option>
          {agent.selectedModel && !models.some((model) => model.id === agent.selectedModel) ? (
            <option value={agent.selectedModel} disabled>{agent.selectedModel} (not in current catalog)</option>
          ) : null}
          {models.map((model) => <option key={model.id} value={model.id} disabled={!model.available}>{model.display_name}</option>)}
        </select>
        {models.length === 0 ? <p>No models reported. Connect a provider to load its model catalog.</p> : null}
        {agent.modelSettings.customModelId ? (
          <div>
            <p>Custom model override takes precedence: <code>{agent.modelSettings.customModelId}</code></p>
            <button type="button" onClick={() => void changePreference(() => agentStore.updateModelSettings({ customModelId: '' }))}>Use selected model instead</button>
          </div>
        ) : null}
        <label htmlFor={`${id}-reasoning`}>Reasoning effort</label>
        <select id={`${id}-reasoning`} value={agent.reasoningEffort} onChange={(event) => {
          const value = event.target.value as ReasoningEffort;
          void changePreference(() => agentStore.setReasoningEffort(value));
        }}>
          <option value="quick">Quick</option>
          <option value="standard">Standard</option>
          <option value="max">Maximum</option>
        </select>
        <label htmlFor={`${id}-instructions`}>Instructions</label>
        <textarea id={`${id}-instructions`} rows={8} value={instructions} onChange={(event) => setInstructionsDraft({ ...instructionsDraft, value: event.target.value })} placeholder="Describe your preferred coding style, constraints, and workflow." aria-describedby={instructionsConflict ? `${id}-instructions-conflict` : undefined} />
        <p>Native agent preferences are saved to agent-preferences.json and shared between windows. Changes apply to subsequent requests.</p>
        {instructionsConflict ? (
          <div>
            <p id={`${id}-instructions-conflict`} role="status">Shared instructions changed while you were editing. Your unsaved draft is preserved. Saving replaces the shared instructions.</p>
            <button type="button" onClick={() => setInstructionsDraft({ value: sharedInstructions, base: sharedInstructions })}>Use shared instructions</button>
          </div>
        ) : null}
        <button type="button" disabled={instructions === agent.modelSettings.systemInstructions} onClick={() => void changePreference(async () => {
          await agentStore.updateModelSettings({ systemInstructions: instructions });
          setInstructionsDraft({ value: instructions, base: instructions });
        })}>Save instructions</button>
      </fieldset>
      <section className="agents-customizations-card" aria-labelledby={`${id}-providers`}>
        <h2 id={`${id}-providers`}>Assistant providers</h2>
        <p>Enable providers in selection lists. Disabling a provider does not disconnect it or change the current model.</p>
        {activeProvider && isProviderDisabled(disabledProviders, activeProvider) ? (
          <p role="status">The active provider is disabled in selection lists. The current model is unchanged.</p>
        ) : null}
        <div className="agents-customizations-providers">
          {PROVIDER_PRESETS.map((provider) => (
            <label key={provider.id} className="agents-customizations-provider">
              <span>{provider.name}</span>
              <input type="checkbox" aria-label={`Enable ${provider.name}`} checked={!isProviderDisabled(disabledProviders, provider.id)} disabled={busy} onChange={(event) => void toggleProvider(provider.id, event.target.checked)} />
            </label>
          ))}
        </div>
        <button type="button" onClick={onOpenConnect} disabled={!onOpenConnect}>Manage provider connection</button>
      </section>
    </section>
  );
}
