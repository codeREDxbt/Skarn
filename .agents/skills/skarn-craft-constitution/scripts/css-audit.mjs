#!/usr/bin/env node
/**
 * Audits Skarn frontend CSS against the Craft Constitution:
 * - transition: all (banned)
 * - *-polish.css files (banned)
 * - banned --glow-*, --glass-*, --grad-* tokens, backdrop-filter, linear/radial-gradient
 * - raw border-radius px values bypassing tokens
 * - !important count
 *
 * Usage:
 *   node css-audit.mjs                 # full audit (exits 0 with summary, or 1 if --strict)
 *   node css-audit.mjs --file <path>   # single-file check (exits 1 on transition: all or *-polish.css)
 *   node css-audit.mjs --strict        # CI gate mode (exits 1 on any banned violation)
 */
import fs from 'node:fs';
import path from 'node:path';

const args = process.argv.slice(2);
const strict = args.includes('--strict');
const fileIdx = args.indexOf('--file');
const singleFile = fileIdx !== -1 ? args[fileIdx + 1] : null;

function walkCss(dir, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (['node_modules', 'dist', 'target', '.git'].includes(entry.name)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walkCss(full, out);
    else if (entry.name.endsWith('.css')) out.push(full);
  }
  return out;
}

function auditFile(filePath) {
  const rel = path.relative(process.cwd(), filePath).replace(/\\/g, '/');
  const text = fs.readFileSync(filePath, 'utf8');
  const lines = text.split(/\r?\n/);
  const findings = {
    rel,
    lines: lines.length,
    isPolish: /-polish\.css$/i.test(rel),
    transitionAll: [],
    bannedEffects: [],
    rawRadius: [],
    important: 0
  };

  lines.forEach((line, i) => {
    const ln = i + 1;
    if (/transition\s*:\s*all\b/i.test(line)) findings.transitionAll.push(ln);
    if (/(--glow-|--glass-|--grad-|backdrop-filter|linear-gradient|radial-gradient)/i.test(line)) {
      findings.bannedEffects.push(ln);
    }
    if (/border-radius\s*:\s*[1-9]\d*px\b/i.test(line) && !line.includes('var(--')) {
      findings.rawRadius.push(ln);
    }
    if (/!important\b/i.test(line)) findings.important += 1;
  });

  return findings;
}

function main() {
  if (singleFile) {
    if (!fs.existsSync(singleFile)) process.exit(0);
    const f = auditFile(singleFile);
    const errs = [];
    if (f.isPolish) errs.push(`${f.rel}: *-polish.css files are banned.`);
    if (f.transitionAll.length) errs.push(`${f.rel}: \`transition: all\` at line(s) ${f.transitionAll.join(', ')}`);
    if (f.rawRadius.length) errs.push(`${f.rel}: raw px border-radius bypass at line(s) ${f.rawRadius.slice(0, 8).join(', ')}`);
    if (errs.length > 0) {
      console.error('CSS Constitution findings:\n' + errs.map((e) => `  - ${e}`).join('\n'));
      process.exit(1);
    }
    process.exit(0);
  }

  const root = ['apps/skarn-desktop/frontend/src', 'apps/selora-desktop/frontend/src'].find((d) => fs.existsSync(d));
  if (!root) {
    console.log('CSS audit: frontend/src not found in cwd.');
    process.exit(0);
  }

  const files = walkCss(root);
  let totalLines = 0;
  let totalTransAll = 0;
  let totalEffects = 0;
  let totalRawRadius = 0;
  let totalImportant = 0;
  const polishFiles = [];

  for (const f of files) {
    const r = auditFile(f);
    totalLines += r.lines;
    totalTransAll += r.transitionAll.length;
    totalEffects += r.bannedEffects.length;
    totalRawRadius += r.rawRadius.length;
    totalImportant += r.important;
    if (r.isPolish) polishFiles.push(r.rel);
  }

  console.log([
    `Skarn CSS Audit (${files.length} files, ${totalLines} total lines):`,
    `  - *-polish.css files : ${polishFiles.length}${polishFiles.length ? ' (' + polishFiles.join(', ') + ')' : ''}`,
    `  - transition: all    : ${totalTransAll}`,
    `  - glow/glass/grad    : ${totalEffects}`,
    `  - raw px radius      : ${totalRawRadius}`,
    `  - !important         : ${totalImportant}`
  ].join('\n'));

  if (strict && (polishFiles.length > 0 || totalTransAll > 0 || totalRawRadius > 0)) {
    console.error('CSS Audit STRICT gate failed.');
    process.exit(1);
  }
}

main();
