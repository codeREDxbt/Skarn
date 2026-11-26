#!/usr/bin/env node
/**
 * Supply-chain & brand-leak gate for Skarn.
 *
 * Default mode (Stage 0+):
 *   Fails on ANY unallowlisted `api.selora.lol` or `ToxicAngelX/Selora-IDE` reference in code.
 *
 * Strict mode (`--strict`, Stage 4/5 release gate):
 *   Also fails on ANY case-insensitive `selora` hit outside the legal/research/v1-migration allowlist.
 */
import fs from 'node:fs';
import path from 'node:path';

const strict = process.argv.includes('--strict');

const SKIP_DIRS = new Set([
  '.git', 'node_modules', 'target', 'dist', 'build', '.agents', '.zcode'
]);

const ALLOWLIST_PATHS = [
  /^LICENSE(\.md)?$/i,
  /^CHANGELOG\.md$/i,
  /^REBRAND_PLAN\.md$/i,
  /^SELORA_.*\.md$/i,
  /^ZCODE_.*\.md$/i,
  /^SKARN_MASTER_.*\.md$/i,
  /^docs\/research\//i,
  /^docs\/brand\//i
];

const TEXT_EXTS = new Set([
  '.rs', '.ts', '.tsx', '.js', '.jsx', '.mjs', '.cjs', '.json', '.toml',
  '.css', '.html', '.iss', '.yml', '.yaml', '.ps1', '.bat', '.sh', '.md', '.txt'
]);

function isAllowlistedFile(rel) {
  return ALLOWLIST_PATHS.some((re) => re.test(rel));
}

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full, out);
    } else {
      const ext = path.extname(entry.name).toLowerCase();
      if (TEXT_EXTS.has(ext) || entry.name === 'LICENSE') out.push(full);
    }
  }
  return out;
}

function main() {
  const files = walk(process.cwd());
  const p0Hits = [];
  const strictHits = [];

  for (const f of files) {
    const rel = path.relative(process.cwd(), f).replace(/\\/g, '/');
    if (isAllowlistedFile(rel)) continue;

    let text;
    try {
      text = fs.readFileSync(f, 'utf8');
    } catch {
      continue;
    }
    const lines = text.split(/\r?\n/);
    const isReadme = /^README\.md$/i.test(rel);

    lines.forEach((line, idx) => {
      const ln = idx + 1;
      if (line.includes('skarn-allow-legacy-fixture')) return;

      if (/api\.selora\.lol/i.test(line)) {
        p0Hits.push(`${rel}:${ln} [legacy-gateway] ${line.trim().slice(0, 120)}`);
      }
      if (/ToxicAngelX\/Selora-IDE/i.test(line)) {
        p0Hits.push(`${rel}:${ln} [legacy-repo-slug] ${line.trim().slice(0, 120)}`);
      }

      if (strict && /selora/i.test(line)) {
        if (line.includes('skarn-v1-migration-compat')) return;
        if (isReadme && /began as Selora IDE/i.test(line)) return;
        strictHits.push(`${rel}:${ln} [legacy-brand] ${line.trim().slice(0, 120)}`);
      }
    });
  }

  if (p0Hits.length > 0) {
    console.error(`P0 Supply-Chain Gate FAILED (${p0Hits.length} hit(s)):\n` + p0Hits.slice(0, 30).map((h) => `  - ${h}`).join('\n'));
    process.exit(1);
  }

  if (strict && strictHits.length > 0) {
    console.error(`Strict Brand-Leak Gate FAILED (${strictHits.length} hit(s)):\n` + strictHits.slice(0, 40).map((h) => `  - ${h}`).join('\n'));
    process.exit(1);
  }

  console.log(`Supply-chain gate OK (${files.length} files scanned${strict ? ', strict mode' : ', P0 mode'}).`);
}

main();
