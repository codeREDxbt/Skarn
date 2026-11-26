#!/usr/bin/env node
/**
 * Antigravity PostToolUse hook for Skarn (Solo Hydrogen).
 * Runs fast, scoped static checks on the file just modified and surfaces concise warnings.
 * Always exits 0 with valid JSON on stdout.
 */
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..', '..');

function readStdin() {
  try {
    return fs.readFileSync(0, 'utf8');
  } catch {
    return '';
  }
}

function respond(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n');
  process.exit(0);
}

function runScript(relScript, extraArgs = []) {
  const full = path.join(repoRoot, relScript);
  if (!fs.existsSync(full)) return null;
  try {
    execFileSync(process.execPath, [full, ...extraArgs], {
      cwd: repoRoot,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout: 12000
    });
    return null;
  } catch (err) {
    const out = [err.stdout, err.stderr].filter(Boolean).join('\n').trim();
    return out ? out.slice(0, 1200) : `Script ${relScript} reported findings.`;
  }
}

function main() {
  let data = {};
  try {
    const raw = readStdin();
    if (raw && raw.trim()) data = JSON.parse(raw);
  } catch {
    respond({});
  }

  if (!data || data.toolCall === null) respond({});

  const args = data?.toolCall?.args || data?.tool_input || data?.args || {};
  const target = String(args.TargetFile || args.file_path || args.path || '').replace(/\\/g, '/');
  if (!target) respond({});

  const warnings = [];

  // 1. Scoped CSS constitution check
  if (target.toLowerCase().endsWith('.css')) {
    const res = runScript('.agents/skills/skarn-craft-constitution/scripts/css-audit.mjs', ['--file', target]);
    if (res) warnings.push(res);
  }

  // 2. Scoped IPC parity check when touching Rust Tauri or TS bridge
  if (
    target.endsWith('bridge.ts') ||
    target.includes('src-tauri/src/')
  ) {
    const res = runScript('.agents/skills/skarn-migration-guard/scripts/check-ipc-parity.mjs');
    if (res) warnings.push(res);
  }

  // 3. Scoped Decision Engine hygiene check
  if (/decision/i.test(target) && (target.endsWith('.rs') || target.endsWith('.ts') || target.endsWith('.tsx'))) {
    const res = runScript('.agents/skills/skarn-decision-invariants/scripts/check-decision-hygiene.mjs', ['--file', target]);
    if (res) warnings.push(res);
  }

  if (warnings.length > 0) {
    respond({
      additionalContext: `[skarn-post-tool-audit] Findings after editing ${target}:\n${warnings.join('\n---\n')}`
    });
  }

  respond({});
}

main();
