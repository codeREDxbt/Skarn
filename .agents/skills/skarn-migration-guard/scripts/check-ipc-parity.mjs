#!/usr/bin/env node
/**
 * Checks that the 5 cross-process Tauri IPC event channels match identically
 * between Rust emit sites and TypeScript listen sites (no split selora/skarn mismatch).
 */
import fs from 'node:fs';
import path from 'node:path';

const CHANNELS = [
  { key: 'event', legacy: 'selora://event', next: 'skarn://event' },
  { key: 'agent-preferences', legacy: 'selora://agent-preferences', next: 'skarn://agent-preferences' },
  { key: 'agents-navigate', legacy: 'selora://agents-navigate', next: 'skarn://agents-navigate' },
  { key: 'main-session-navigate', legacy: 'selora://main-session-navigate', next: 'skarn://main-session-navigate' },
  { key: 'update-progress', legacy: 'selora:update-progress', next: 'skarn:update-progress' }
];

function walk(dir, exts, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (['node_modules', 'target', 'dist', '.git', '.agents'].includes(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, exts, out);
    else if (exts.some((e) => entry.name.endsWith(e))) out.push(full);
  }
  return out;
}

function scanFiles(files, needle) {
  const hits = [];
  for (const f of files) {
    const content = fs.readFileSync(f, 'utf8');
    if (content.includes(needle)) hits.push(path.relative(process.cwd(), f).replace(/\\/g, '/'));
  }
  return hits;
}

function main() {
  const appDir = ['apps/skarn-desktop', 'apps/selora-desktop'].find((d) => fs.existsSync(d));
  if (!appDir) {
    console.log('IPC parity: skipped (desktop app directory not found in cwd).');
    process.exit(0);
  }

  const rustFiles = walk(path.join(appDir, 'src-tauri/src'), ['.rs']);
  const tsFiles = walk(path.join(appDir, 'frontend/src'), ['.ts', '.tsx']);

  const errors = [];
  for (const ch of CHANNELS) {
    const rustLegacy = scanFiles(rustFiles, ch.legacy);
    const rustNext = scanFiles(rustFiles, ch.next);
    const tsLegacy = scanFiles(tsFiles, ch.legacy);
    const tsNext = scanFiles(tsFiles, ch.next);

    const rustUses = (rustLegacy.length ? 'legacy' : '') + (rustNext.length ? '+next' : '');
    const tsUses = (tsLegacy.length ? 'legacy' : '') + (tsNext.length ? '+next' : '');

    if (rustUses !== tsUses) {
      errors.push(
        `Channel "${ch.key}" MISMATCH: Rust uses [${rustUses || 'none'}] (${[...rustLegacy, ...rustNext].join(', ') || 'none'}) vs TS uses [${tsUses || 'none'}] (${[...tsLegacy, ...tsNext].join(', ') || 'none'})`
      );
    }
  }

  if (errors.length > 0) {
    console.error('IPC Channel Parity FAILED:\n' + errors.map((e) => `  - ${e}`).join('\n'));
    process.exit(1);
  }

  console.log('IPC Channel Parity OK (all 5 Rust <-> TS channels aligned).');
}

main();
