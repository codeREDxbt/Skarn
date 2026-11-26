#!/usr/bin/env node
/**
 * Antigravity PreToolUse hook for Skarn (Solo Hydrogen).
 * - Blocks destructive git history rewrites and live user-profile/keyring tampering.
 * - Prevents the agent from disabling or editing .agents/hooks*.
 * - Blocks introducing legacy gateway URLs, banned CSS patterns, or raw API keys.
 *
 * Contract: stdout MUST contain ONLY a single JSON object; always exit 0.
 */
import fs from 'node:fs';
import path from 'node:path';

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

function deny(reason) {
  respond({ decision: 'deny', reason: `[skarn-pre-tool-guard] ${reason}` });
}

function allow() {
  respond({ decision: 'allow' });
}

function extractPayload(raw) {
  if (!raw || !raw.trim()) return {};
  try {
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

function getToolName(data) {
  return String(
    data?.toolCall?.name ||
    data?.tool_name ||
    data?.toolName ||
    data?.name ||
    ''
  );
}

function getToolArgs(data) {
  return (
    data?.toolCall?.args ||
    data?.tool_input ||
    data?.toolInput ||
    data?.args ||
    {}
  );
}

function normalizePath(p) {
  return String(p || '').replace(/\\/g, '/');
}

function isDocOrFixturePath(relPath) {
  const lower = relPath.toLowerCase();
  if (
    lower.endsWith('rebrand_plan.md') ||
    lower.endsWith('changelog.md') ||
    lower.endsWith('license') ||
    lower.endsWith('license.md') ||
    lower.includes('selora_master_dossier.md') ||
    lower.includes('selora_ux_painpoints.md') ||
    lower.includes('selora_craft_spec.md') ||
    lower.includes('zcode_ux_teardown.md') ||
    lower.includes('zcode_parity_prompts.md') ||
    lower.includes('skarn_master_') ||
    lower.includes('docs/research/') ||
    lower.includes('docs/brand/') ||
    lower.includes('.test.') ||
    lower.includes('.spec.') ||
    lower.includes('/tests/') ||
    lower.includes('__tests__') ||
    lower.endsWith('_test.rs')
  ) {
    return true;
  }
  return false;
}

function collectWrittenText(args) {
  const chunks = [];
  for (const key of ['CodeContent', 'ReplacementContent', 'content', 'new_string', 'newText', 'text']) {
    if (typeof args[key] === 'string') chunks.push(args[key]);
  }
  if (Array.isArray(args.ReplacementChunks)) {
    for (const c of args.ReplacementChunks) {
      if (typeof c?.ReplacementContent === 'string') chunks.push(c.ReplacementContent);
    }
  }
  return chunks.join('\n');
}

function main() {
  const data = extractPayload(readStdin());
  const toolName = getToolName(data);
  const args = getToolArgs(data);

  // 1. Shell command guard
  if (toolName === 'run_command' || toolName === 'bash' || toolName === 'shell') {
    const cmd = String(args.CommandLine || args.command || args.cmd || '');
    const blockedCmds = [
      { re: /\bgit\s+push\b[^\n]*(--force|-f\b|--force-with-lease)/i, msg: 'Force-pushing is prohibited by Skarn policy.' },
      { re: /\bgit\s+(filter-branch|filter-repo)\b/i, msg: 'Rewriting git history is prohibited (REBRAND_PLAN §6.8).' },
      { re: /\bgit\s+reset\s+--hard\b/i, msg: 'Hard git reset is blocked; use targeted file restores or ask the user.' },
      { re: /\bcmdkey\s+\/delete\b/i, msg: 'Direct deletion of Windows Credential Manager entries via shell is blocked outside isolated tests.' },
      { re: /(%APPDATA%|\$env:APPDATA|~\/\.config)[^\n]*(rm\s+-rf|Remove-Item|del\s+\/)/i, msg: 'Deleting real user %APPDATA% settings directories is blocked. Use a temp directory fixture.' }
    ];
    for (const rule of blockedCmds) {
      if (rule.re.test(cmd)) deny(rule.msg);
    }
    allow();
  }

  // 2. File write / edit guard
  const target = normalizePath(
    args.TargetFile || args.file_path || args.path || args.filePath || ''
  );
  if (!target) allow();

  // Protect hook definitions and scripts from agent self-modification
  if (
    /(^|\/)\.agents\/hooks\.json$/i.test(target) ||
    /(^|\/)\.agents\/hooks\//i.test(target)
  ) {
    deny('Modifying .agents/hooks.json or .agents/hooks/* is restricted to the human owner.');
  }

  // Block creating new *-polish.css files (Craft Constitution)
  if (/-polish\.css$/i.test(target)) {
    deny('Creating or editing *-polish.css files violates the Skarn Craft Constitution. Consolidate styles into the base stylesheet instead.');
  }

  const content = collectWrittenText(args);
  if (!content) allow();

  // Block new `transition: all` in CSS files
  if (target.toLowerCase().endsWith('.css')) {
    if (/transition\s*:\s*all\b/i.test(content)) {
      deny('`transition: all` is banned by the Skarn Craft Constitution. Animate explicit properties (opacity, transform, color, background-color, border-color).');
    }
  }

  // Skip legacy/secret string checks for research docs, LICENSE, CHANGELOG, and explicit unit test fixtures
  if (!isDocOrFixturePath(target)) {
    const lines = content.split(/\r?\n/);
    for (const line of lines) {
      if (line.includes('skarn-allow-legacy-fixture')) continue;

      if (/api\.selora\.lol/i.test(line)) {
        deny('Legacy gateway URL `api.selora.lol` cannot be written into source code (Stage 0 / P0 invariant).');
      }
      if (/ToxicAngelX\/Selora-IDE/i.test(line)) {
        deny('Hardcoded legacy repo slug `ToxicAngelX/Selora-IDE` is forbidden (Stage 0 supply-chain invariant). Add `// skarn-allow-legacy-fixture` only inside an isolation unit test.');
      }
      if (/\b(sk-selora-[A-Za-z0-9_-]{8,}|sk-gw-[A-Za-z0-9_-]{8,}|AIzaSy[A-Za-z0-9_-]{20,})\b/.test(line)) {
        deny('Potential raw API key detected in file edit. Store secrets strictly in OS Credential Manager.');
      }
    }
  }

  allow();
}

main();
