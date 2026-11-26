#!/usr/bin/env node
/**
 * Rewrites command paths in .agents/hooks.json to absolute paths based on the repo root.
 * Prevents "exit 127 / script not found" if Antigravity CLI is launched from a subdirectory.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const agentsDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(agentsDir, '..');
const hooksJsonPath = path.join(agentsDir, 'hooks.json');

const raw = fs.readFileSync(hooksJsonPath, 'utf8');
const config = JSON.parse(raw);

function rewriteCommand(cmd) {
  if (typeof cmd !== 'string') return cmd;
  return cmd.replace(/\.agents\/hooks\/([a-zA-Z0-9._-]+\.mjs)/g, (_m, file) => {
    const abs = path.join(repoRoot, '.agents', 'hooks', file).replace(/\\/g, '/');
    return abs.includes(' ') ? `"${abs}"` : abs;
  });
}

for (const hookDef of Object.values(config)) {
  if (!hookDef || typeof hookDef !== 'object') continue;
  for (const event of ['PreToolUse', 'PostToolUse']) {
    if (!Array.isArray(hookDef[event])) continue;
    for (const entry of hookDef[event]) {
      if (!Array.isArray(entry.hooks)) continue;
      for (const h of entry.hooks) {
        h.command = rewriteCommand(h.command);
      }
    }
  }
  for (const event of ['SessionStart', 'PreInvocation', 'PostInvocation', 'Stop']) {
    if (!Array.isArray(hookDef[event])) continue;
    for (const h of hookDef[event]) {
      h.command = rewriteCommand(h.command);
    }
  }
}

fs.writeFileSync(hooksJsonPath, JSON.stringify(config, null, 2) + '\n', 'utf8');
console.log(`Updated ${hooksJsonPath} with absolute hook paths rooted at ${repoRoot}`);
