// Font-scale contract: the runtime writes `data-font-scale` on the root and
// tokens.css turns that flag into the coarse `--font-scale` multipliers.
import { describe, expect, it } from 'vitest';

// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { readFileSync } from 'node:fs';
// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { join } from 'node:path';

const TOKENS = readFileSync(
  join((globalThis as unknown as { process: { cwd(): string } }).process.cwd(), 'src/styles/tokens.css'),
  'utf8',
);

describe('font scale tokens', () => {
  it('defaults to 1 and scales the type ramp through one variable', () => {
    expect(TOKENS).toContain('--font-scale: 1;');
    expect(TOKENS).toContain('--type-xs: calc(11px * var(--font-scale));');
    expect(TOKENS).toContain('--type-md: calc(13px * var(--font-scale));');
    expect(TOKENS).toContain('--type-xxl-lh: calc(32px * var(--font-scale));');
  });

  it('declares one hook per step the runtime can write', () => {
    expect(TOKENS).toContain(':root[data-font-scale="large"]');
    expect(TOKENS).toContain(':root[data-font-scale="x-large"]');
  });
});