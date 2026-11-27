import React, { useState } from 'react';
import { ChevronDown, ChevronRight, ShieldCheck } from '../../../components/icons';
import type { DecisionPhaseDto, DecisionTraceDto, ExecutionTierDto } from '../../../bridge/bridge';

export type DecisionEvidenceTrust = 'verified_fact' | 'untrusted_workspace' | 'inferred';

export type DecisionEvidenceItem = {
  source_ref: string;
  trust: DecisionEvidenceTrust;
  summary?: string;
};

export type DecisionTraceDrawerProps = {
  trace: DecisionTraceDto;
  evidence?: DecisionEvidenceItem[];
  defaultOpen?: boolean;
};

function scrubSecretText(raw: string): string {
  return raw
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, '[REDACTED]')
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]{6,}/gi, 'Bearer [REDACTED]')
    .replace(/\bAuthorization\s*:\s*\S+/gi, 'Authorization: [REDACTED]');
}

function tierLabel(tier: ExecutionTierDto): string {
  switch (tier) {
    case 'tier1_deterministic':
      return 'Tier 1 Deterministic';
    case 'tier2_decision_model':
      return 'Tier 2 Decision';
    case 'tier3_general_llm':
      return 'Tier 3 LLM';
  }
}

function snapshotStatusLabel(fsmPath: readonly DecisionPhaseDto[], phase: DecisionPhaseDto): string {
  if (fsmPath.includes('stale') || phase === 'stale') return 'Stale (drifted)';
  if (fsmPath.includes('snapshot_rechecked')) return 'Rechecked OK';
  if (fsmPath.includes('snapshotted')) return 'Snapshotted';
  return 'Not snapshotted';
}

function deriveEvidenceTags(trace: DecisionTraceDto): DecisionEvidenceItem[] {
  const derived: DecisionEvidenceItem[] = [];
  if (trace.fsm_path.includes('snapshot_rechecked') || trace.fsm_path.includes('snapshotted')) {
    derived.push({
      source_ref: `snapshot:${trace.action_id}`,
      trust: 'verified_fact',
      summary: snapshotStatusLabel(trace.fsm_path, trace.phase),
    });
  }
  if (trace.fsm_path.includes('policy_evaluated')) {
    derived.push({
      source_ref: `policy:${trace.reason_code}`,
      trust: 'verified_fact',
      summary: `Outcome ${trace.phase}`,
    });
  }
  if (trace.fsm_path.includes('inferred') || trace.selected_tier !== 'tier1_deterministic') {
    derived.push({
      source_ref: trace.model_id ? `model:${trace.model_id}` : `tier:${trace.selected_tier}`,
      trust: 'inferred',
      summary: `Hops: ${trace.hop_count}`,
    });
  }
  return derived;
}

export const DecisionTraceDrawer: React.FC<DecisionTraceDrawerProps> = ({
  trace,
  evidence,
  defaultOpen = false,
}) => {
  const [open, setOpen] = useState(defaultOpen);
  const items = evidence && evidence.length > 0 ? evidence : deriveEvidenceTags(trace);
  const snapshotStatus = snapshotStatusLabel(trace.fsm_path, trace.phase);

  return (
    <div className="decision-trace-drawer" data-testid="decision-trace-drawer">
      <button
        type="button"
        className="decision-trace-toggle"
        aria-expanded={open}
        onClick={(event) => {
          event.stopPropagation();
          setOpen((prev) => !prev);
        }}
      >
        {open ? <ChevronDown size={11} aria-hidden="true" /> : <ChevronRight size={11} aria-hidden="true" />}
        <ShieldCheck size={11} aria-hidden="true" />
        <span>Decision trace</span>
        <span className="decision-trace-pill">{tierLabel(trace.selected_tier)}</span>
        <span className={`decision-trace-pill phase-${trace.phase}`}>{trace.phase}</span>
      </button>

      {open && (
        <div className="decision-trace-body">
          <div className="decision-trace-grid">
            <div className="decision-trace-row">
              <span className="decision-trace-label">FSM Path</span>
              <span className="decision-trace-value" data-testid="decision-trace-fsm">
                {trace.fsm_path.join(' → ')}
              </span>
            </div>
            <div className="decision-trace-row">
              <span className="decision-trace-label">Tier / Provider</span>
              <span className="decision-trace-value">
                {tierLabel(trace.selected_tier)}
                {trace.provider_id ? ` · ${scrubSecretText(trace.provider_id)}` : ''}
                {trace.model_id ? ` (${scrubSecretText(trace.model_id)})` : ''}
                {` · hops: ${trace.hop_count}`}
              </span>
            </div>
            <div className="decision-trace-row">
              <span className="decision-trace-label">Reason Code</span>
              <span className="decision-trace-value">{scrubSecretText(trace.reason_code)}</span>
            </div>
            <div className="decision-trace-row">
              <span className="decision-trace-label">Snapshot Status</span>
              <span className="decision-trace-value">{snapshotStatus}</span>
            </div>
            <div className="decision-trace-row">
              <span className="decision-trace-label">Policy Outcome</span>
              <span className="decision-trace-value">
                {trace.phase} · risk: {trace.risk}
                {trace.privacy_blocked_cloud ? ' · cloud blocked (local-only)' : ''}
              </span>
            </div>
            {(trace.raw_confidence !== undefined && trace.raw_confidence !== null) ||
            (trace.calibrated_threshold !== undefined && trace.calibrated_threshold !== null) ? (
              <div className="decision-trace-row">
                <span className="decision-trace-label">Calibration</span>
                <span className="decision-trace-value">
                  {trace.raw_confidence !== undefined && trace.raw_confidence !== null
                    ? `raw ${trace.raw_confidence.toFixed(2)}`
                    : 'raw —'}
                  {trace.calibrated_threshold !== undefined && trace.calibrated_threshold !== null
                    ? ` / min ${trace.calibrated_threshold.toFixed(2)}`
                    : ''}
                </span>
              </div>
            ) : null}
          </div>

          {items.length > 0 && (
            <div className="decision-trace-evidence" aria-label="Evidence trust tags">
              <span className="decision-trace-label">Evidence Trust</span>
              <ul className="decision-trace-evidence-list">
                {items.map((item, idx) => (
                  <li key={`${item.source_ref}-${idx}`} className="decision-trace-evidence-item">
                    <span className={`decision-evidence-trust trust-${item.trust}`}>
                      {item.trust}
                    </span>
                    <span className="decision-evidence-ref">
                      {scrubSecretText(item.source_ref)}
                    </span>
                    {item.summary && (
                      <span className="decision-evidence-summary">
                        {scrubSecretText(item.summary)}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
