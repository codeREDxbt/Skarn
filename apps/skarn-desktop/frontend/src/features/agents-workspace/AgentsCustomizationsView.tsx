import { useEffect, useId, useState, useSyncExternalStore } from 'react';
import type { BootstrapState } from '../../bridge/bridge';
import { agentStore } from '../agent/agent-store';
import './agents-customizations.css';

export type AgentsCustomizationsViewProps = {
  bootstrap?: BootstrapState;
  onOpenConnect?: () => void;
};

/**
 * Single surviving Customizations scope: global agent instructions saved in
 * `agent-preferences.json`. Providers live in `Settings -> Providers`, and
 * model + reasoning effort live in the Composer.
 */
export function AgentsCustomizationsView(_props: AgentsCustomizationsViewProps) {
  const agent = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const id = useId();
  const sharedInstructions = agent.modelSettings.systemInstructions;
  const [instructionsDraft, setInstructionsDraft] = useState({ value: sharedInstructions, base: sharedInstructions });
  const instructions = instructionsDraft.value;
  const instructionsConflict = instructionsDraft.base !== sharedInstructions && instructions !== sharedInstructions;
  const [feedback, setFeedback] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setInstructionsDraft((draft) => (
      draft.value === draft.base || draft.value === sharedInstructions
        ? { value: sharedInstructions, base: sharedInstructions }
        : draft
    ));
  }, [sharedInstructions]);

  const saveInstructions = async () => {
    setFeedback(null);
    setError(null);
    setBusy(true);
    try {
      await agentStore.updateModelSettings({ systemInstructions: instructions });
      setInstructionsDraft({ value: instructions, base: instructions });
      setFeedback('Instructions saved for subsequent requests.');
    } catch (cause) {
      setError(`Preference update failed: ${cause instanceof Error ? cause.message : String(cause)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="agents-customizations" aria-labelledby={`${id}-title`}>
      <header>
        <h1 id={`${id}-title`}>Customizations</h1>
        <p>Global agent instructions. Changes apply to subsequent requests, not an ongoing turn.</p>
      </header>
      {error ? <p className="agents-customizations-error" role="alert">{error}</p> : null}
      {feedback ? <p role="status">{feedback}</p> : null}
      <fieldset className="agents-customizations-card" disabled={busy}>
        <legend>Global instructions</legend>
        <label htmlFor={`${id}-instructions`}>Instructions</label>
        <textarea
          id={`${id}-instructions`}
          rows={8}
          value={instructions}
          onChange={(event) => setInstructionsDraft({ ...instructionsDraft, value: event.target.value })}
          placeholder="Describe your preferred coding style, constraints, and workflow."
          aria-describedby={instructionsConflict ? `${id}-instructions-conflict` : undefined}
        />
        <p>Saved to agent-preferences.json and shared between windows. Changes apply to subsequent requests.</p>
        {instructionsConflict ? (
          <div>
            <p id={`${id}-instructions-conflict`} role="status">Shared instructions changed while you were editing. Your unsaved draft is preserved. Saving replaces the shared instructions.</p>
            <button type="button" onClick={() => setInstructionsDraft({ value: sharedInstructions, base: sharedInstructions })}>Use shared instructions</button>
          </div>
        ) : null}
        <button
          type="button"
          disabled={instructions === agent.modelSettings.systemInstructions}
          onClick={() => void saveInstructions()}
        >
          Save instructions
        </button>
      </fieldset>
    </section>
  );
}
