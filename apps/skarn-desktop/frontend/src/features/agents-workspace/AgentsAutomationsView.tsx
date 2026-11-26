import { WorkflowsPanel } from '../workflows/WorkflowsPanel';
import type { WorkflowsStore } from '../workflows/workflows-store';
import './agents-customizations.css';

export type AgentsAutomationsViewProps = { store?: WorkflowsStore };

export function AgentsAutomationsView({ store }: AgentsAutomationsViewProps) {
  return (
    <section className="agents-automations" aria-label="Automations">
      <header>
        <h1>Automations</h1>
        <p>Follow approved workflow runs, review approvals, and inspect results. Start workflows by approving a workflow script in chat.</p>
      </header>
      <WorkflowsPanel store={store} />
    </section>
  );
}
