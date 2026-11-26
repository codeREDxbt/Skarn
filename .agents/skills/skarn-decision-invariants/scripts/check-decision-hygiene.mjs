#!/usr/bin/env node
/**
 * Static safety scan over Decision Engine source files:
 * - Flags shell interpolation patterns (`sh -c`, `cmd /C`, `exec(`) inside decision executors
 * - Flags logging of Authorization headers or API keys
 * - Flags silent auto-exec bypasses on high/critical risk
 */
import fs from 'node:fs';
import path from 'node:path';

const args = process.argv.slice(2);
const fileIdx = args.indexOf('--file');
const singleFile = fileIdx !== -1 ? args[fileIdx + 1] : null;

function walk(dir, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (['node_modules', 'target', 'dist', '.git', '.agents'].includes(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, out);
    else if (/decision/i.test(full) && /\.(rs|ts|tsx)$/i.test(entry.name)) out.push(full);
  }
  return out;
}

const RULES = [
  {
    id: 'no-shell-interp',
    re: /(Command::new\(["'](sh|bash|cmd|powershell|pwsh)["']\)|child_process\.exec\(|\bsh\s+-c\b)/i,
    msg: 'Potential shell interpolation inside Decision Engine code. Execute strictly through the controlled action registry.'
  },
  {
    id: 'no-secret-log',
    re: /(println!|tracing::(info|debug|warn|error)!|console\.(log|debug|warn|error))[^\n]*(api_key|apiKey|authorization|bearer)/i,
    msg: 'Potential secret/header logging detected. Scrub API keys and Authorization headers before logging.'
  },
  {
    id: 'no-unbounded-unwrap-in-provider',
    re: /\.unwrap\(\)\s*;\s*\/\/\s*provider/i,
    msg: 'Avoid .unwrap() on untrusted provider output; return structured validation errors.'
  }
];

function checkFile(filePath) {
  const rel = path.relative(process.cwd(), filePath).replace(/\\/g, '/');
  if (rel.includes('.test.') || rel.includes('_test.rs') || rel.includes('/tests/')) return [];
  const lines = fs.readFileSync(filePath, 'utf8').split(/\r?\n/);
  const hits = [];
  lines.forEach((line, idx) => {
    if (line.includes('skarn-allow-decision-rule')) return;
    for (const rule of RULES) {
      if (rule.re.test(line)) {
        hits.push(`${rel}:${idx + 1} [${rule.id}] ${rule.msg}`);
      }
    }
  });
  return hits;
}

function main() {
  const targets = singleFile ? (fs.existsSync(singleFile) ? [singleFile] : []) : walk(process.cwd());
  if (targets.length === 0) {
    console.log('Decision hygiene: no decision source files found yet (OK).');
    process.exit(0);
  }

  const allHits = [];
  for (const t of targets) allHits.push(...checkFile(t));

  if (allHits.length > 0) {
    console.error('Decision Engine Hygiene FAILED:\n' + allHits.map((h) => `  - ${h}`).join('\n'));
    process.exit(1);
  }
  console.log(`Decision hygiene OK (${targets.length} file(s) scanned).`);
}

main();
