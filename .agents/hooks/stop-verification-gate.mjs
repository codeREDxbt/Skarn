#!/usr/bin/env node
/**
 * Antigravity Stop hook for Skarn (Solo Hydrogen).
 * Acts as a zero-token pre-completion reviewer:
 * - Immediately allows stop on API errors / quota exhaustion (prevents 429 infinite loops).
 * - Immediately allows stop on read-only / plan turns (no modified code files).
 * - Caps consecutive stop-blocks at 2 per 5-minute window to protect solo-Hydrogen quota.
 * - Runs scoped verification: legacy supply-chain check, cargo check (if .rs edited), npm typecheck (if .ts/.tsx edited).
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync, execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..', '..');

function respond(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n');
  process.exit(0);
}

function readStdinJson() {
  try {
    const raw = fs.readFileSync(0, 'utf8');
    return raw && raw.trim() ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

function getRetryState() {
  const hash = crypto.createHash('sha1').update(repoRoot).digest('hex').slice(0, 12);
  const file = path.join(os.tmpdir(), `skarn-stop-gate-${hash}.json`);
  try {
    const parsed = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (Date.now() - parsed.ts < 5 * 60 * 1000) {
      return { file, count: parsed.count || 0 };
    }
  } catch {
    // fresh window
  }
  return { file, count: 0 };
}

function saveRetryState(file, count) {
  try {
    fs.writeFileSync(file, JSON.stringify({ ts: Date.now(), count }), 'utf8');
  } catch {
    // ignore
  }
}

function clearRetryState(file) {
  try {
    if (fs.existsSync(file)) fs.unlinkSync(file);
  } catch {
    // ignore
  }
}

function getChangedFiles() {
  try {
    const out = execSync('git status --porcelain', {
      cwd: repoRoot,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
      timeout: 5000
    });
    return out
      .split(/\r?\n/)
      .filter(Boolean)
      .map((line) => line.slice(3).trim().replace(/^"|"$/g, '').replace(/\\/g, '/'));
  } catch {
    return [];
  }
}

function findFrontendDir() {
  for (const candidate of [
    'apps/skarn-desktop/frontend',
    'apps/selora-desktop/frontend'
  ]) {
    const full = path.join(repoRoot, candidate);
    if (fs.existsSync(path.join(full, 'package.json'))) return full;
  }
  return null;
}

function runCheck(label, cmd, args, cwd, timeoutMs = 90000) {
  try {
    execFileSync(cmd, args, {
      cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout: timeoutMs,
      shell: process.platform === 'win32'
    });
    return null;
  } catch (err) {
    const msg = [err.stdout, err.stderr].filter(Boolean).join('\n').trim();
    const tail = msg.split(/\r?\n/).slice(-35).join('\n');
    return `### ${label} FAILED\n${tail || err.message}`;
  }
}

function main() {
  const payload = readStdinJson();

  // 1. Never trap quota exhaustion, network errors, or user aborts in a retry loop
  if (payload.error && String(payload.error).trim() !== '') {
    respond({ decision: 'stop' });
  }
  const term = String(payload.terminationReason || '');
  if (term && !['NO_TOOL_CALL', 'END_TURN', 'STOP', 'COMPLETE'].includes(term.toUpperCase())) {
    respond({ decision: 'stop' });
  }

  // 2. Skip verification on read-only / planning sessions
  const changed = getChangedFiles();
  const codeChanged = changed.filter((f) => /\.(rs|ts|tsx|css|iss|toml|json)$/i.test(f) && !f.startsWith('.agents/'));
  if (codeChanged.length === 0) {
    respond({ decision: 'stop' });
  }

  // 3. Quota protection: allow stop after 2 consecutive automatic retries
  const retry = getRetryState();
  if (retry.count >= 2) {
    clearRetryState(retry.file);
    respond({ decision: 'stop' });
  }

  const failures = [];

  // A. P0 Supply-chain & legacy gateway check (fast, always runs on code edits)
  const legacyScript = path.join(repoRoot, '.agents/skills/skarn-supply-chain-gate/scripts/check-legacy-refs.mjs');
  if (fs.existsSync(legacyScript)) {
    const res = runCheck('Supply-Chain Legacy Gate', process.execPath, [legacyScript], repoRoot, 15000);
    if (res) failures.push(res);
  }

  // B. Rust check if any .rs or Cargo.toml file changed
  const rustTouched = codeChanged.some((f) => f.endsWith('.rs') || f.endsWith('Cargo.toml'));
  if (rustTouched && fs.existsSync(path.join(repoRoot, 'Cargo.toml'))) {
    const res = runCheck('cargo check --workspace', 'cargo', ['check', '--workspace', '--quiet'], repoRoot, 90000);
    if (res) failures.push(res);
  }

  // C. Frontend typecheck if any .ts or .tsx file changed
  const tsTouched = codeChanged.some((f) => f.endsWith('.ts') || f.endsWith('.tsx'));
  const frontendDir = findFrontendDir();
  if (tsTouched && frontendDir) {
    const res = runCheck('npm run typecheck', 'npm', ['run', 'typecheck'], frontendDir, 60000);
    if (res) failures.push(res);
  }

  if (failures.length > 0) {
    saveRetryState(retry.file, retry.count + 1);
    respond({
      decision: 'continue',
      reason: `[skarn-stop-gate] Verification failed (${retry.count + 1}/2). Fix these errors before completing your turn:\n\n${failures.join('\n\n')}`
    });
  }

  clearRetryState(retry.file);
  respond({ decision: 'stop' });
}

main();
