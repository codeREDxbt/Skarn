// Galaxy palette contract.
//
// The Galaxy theme is opt-in and CSS-only, so the guarantees live in the
// stylesheet: every token the default dark ramp exposes must be re-declared
// for `[data-theme="galaxy"]`, once, and the text/border pairs must stay
// readable. `tokens.css` is the source of truth for the token list; this test
// reads both files from disk instead of importing them so it cannot be fooled
// by cascade order.
import { describe, expect, it } from 'vitest';
import { GALAXY_THEME, isGalaxyTheme } from './galaxy';

// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { readFileSync } from 'node:fs';
// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { join } from 'node:path';

const CWD = (globalThis as unknown as { process: { cwd(): string } }).process.cwd();
const TOKENS_CSS = readFileSync(join(CWD, 'src/styles/tokens.css'), 'utf8');
const GALAXY_CSS = readFileSync(join(CWD, 'src/styles/theme-galaxy.css'), 'utf8');

interface Declaration {
  name: string;
  value: string;
}

const DECLARATION = /(--[a-z0-9-]+)\s*:\s*([^;]+);/gi;

/** Return the declarations inside the first block whose selector starts with `header`. */
function declarationsIn(css: string, header: string): Declaration[] {
  const selector = css.indexOf(header);
  if (selector === -1) throw new Error(`missing selector: ${header}`);
  const open = css.indexOf('{', selector);
  if (open === -1) throw new Error(`missing block for: ${header}`);

  let depth = 0;
  for (let index = open; index < css.length; index += 1) {
    if (css[index] === '{') depth += 1;
    else if (css[index] === '}') {
      depth -= 1;
      if (depth === 0) return declarations(css.slice(open + 1, index));
    }
  }
  throw new Error(`unterminated block for: ${header}`);
}

function declarations(css: string): Declaration[] {
  return [...css.matchAll(DECLARATION)].map((match) => ({
    name: match[1],
    value: match[2].trim(),
  }));
}

/** The default dark palette: the first `:root` block in tokens.css. */
const BASE_DECLARATIONS = declarationsIn(TOKENS_CSS, ':root {');
const BASE_NAMES = [...new Set(BASE_DECLARATIONS.map(({ name }) => name))];

const GALAXY_SELECTOR = '[data-theme="galaxy"] {';
const GALAXY_BLOCK = declarationsIn(GALAXY_CSS, GALAXY_SELECTOR);
const GALAXY_VALUES = new Map(GALAXY_BLOCK.map(({ name, value }) => [name, value]));

interface Rgb {
  r: number;
  g: number;
  b: number;
}

function expand(hex: string): Rgb {
  const full = hex.length === 3 ? hex.split('').map((char) => char + char).join('') : hex;
  return {
    r: Number.parseInt(full.slice(0, 2), 16),
    g: Number.parseInt(full.slice(2, 4), 16),
    b: Number.parseInt(full.slice(4, 6), 16),
  };
}

/** Parse the first color in a value (hex or rgb/rgba) after resolving `var()`. */
function parseColor(value: string): Rgb {
  const hex = /#([0-9a-f]{6}|[0-9a-f]{3})\b/i.exec(value);
  if (hex) return expand(hex[1]);

  const rgb = /rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(value);
  if (rgb) return { r: Number(rgb[1]), g: Number(rgb[2]), b: Number(rgb[3]) };

  throw new Error(`not a color: ${value}`);
}

/** Read a galaxy token, following one level of `var(--token)` references. */
function galaxyColor(name: string): Rgb {
  const raw = GALAXY_VALUES.get(name);
  if (!raw) throw new Error(`galaxy does not declare ${name}`);
  const resolved = raw.replace(/var\((--[a-z0-9-]+)\)/gi, (_match, reference: string) => {
    const target = GALAXY_VALUES.get(reference);
    if (!target) throw new Error(`galaxy ${name} references undeclared ${reference}`);
    return target;
  });
  return parseColor(resolved);
}

function channel(value: number): number {
  const scaled = value / 255;
  return scaled <= 0.03928 ? scaled / 12.92 : ((scaled + 0.055) / 1.055) ** 2.4;
}

function luminance({ r, g, b }: Rgb): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(foreground: Rgb, background: Rgb): number {
  const [lighter, darker] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

describe('galaxy palette coverage', () => {
  it('declares the galaxy selector exactly once', () => {
    expect(GALAXY_CSS.split('[data-theme="galaxy"]').length - 1).toBe(1);
    expect(GALAXY_BLOCK.length).toBeGreaterThan(0);
  });

  it('re-declares every custom property from the default dark ramp', () => {
    const missing = BASE_NAMES.filter((name) => !GALAXY_VALUES.has(name));
    expect(missing).toEqual([]);
    expect(BASE_NAMES.length).toBeGreaterThan(100);
  });

  it('declares every custom property exactly once', () => {
    const seen = new Map<string, number>();
    for (const { name } of declarations(GALAXY_CSS)) {
      seen.set(name, (seen.get(name) ?? 0) + 1);
    }
    const duplicates = [...seen].filter(([, count]) => count > 1).map(([name]) => name);
    expect(duplicates).toEqual([]);
  });

  it('keeps every declaration inside the galaxy selector', () => {
    const selectorStart = GALAXY_CSS.indexOf(GALAXY_SELECTOR);
    const blockOpen = GALAXY_CSS.indexOf('{', selectorStart);
    const blockClose = GALAXY_CSS.lastIndexOf('}');
    const outside = `${GALAXY_CSS.slice(0, blockOpen + 1)}${GALAXY_CSS.slice(blockClose)}`;
    expect(declarations(outside)).toEqual([]);
  });
});

describe('galaxy contrast', () => {
  const textPairs: ReadonlyArray<readonly [string, string, number]> = [
    ['--fg-default', '--ink-canvas', 4.5],
    ['--fg-default', '--ink-low', 4.5],
    ['--fg-default', '--ink-mid', 4.5],
    ['--fg-default', '--surface-3', 4.5],
    ['--fg-bright', '--ink-mid', 4.5],
    ['--fg-muted', '--ink-low', 4.5],
    ['--fg-muted', '--ink-mid', 4.5],
    ['--fg-faint', '--ink-canvas', 4.5],
    ['--fg-faint', '--ink-low', 4.5],
    ['--syn-default', '--ink-canvas', 4.5],
    ['--syn-keyword', '--ink-canvas', 4.5],
    ['--syn-fn', '--ink-canvas', 4.5],
    ['--syn-type', '--ink-canvas', 4.5],
    ['--syn-string', '--ink-canvas', 4.5],
    ['--syn-number', '--ink-canvas', 4.5],
    ['--syn-comment', '--ink-low', 4.5],
    ['--syn-punct', '--ink-low', 4.5],
    ['--syn-attr', '--ink-canvas', 4.5],
    ['--ok', '--ink-low', 4.5],
    ['--warn', '--ink-low', 4.5],
    ['--err', '--ink-low', 4.5],
    ['--info', '--ink-low', 4.5],
    ['--diag-err', '--ink-low', 4.5],
    ['--diag-warn', '--ink-low', 4.5],
    ['--diag-info', '--ink-low', 4.5],
  ];

  it.each(textPairs)('keeps %s readable on %s (>= %s:1)', (foreground, background, minimum) => {
    const ratio = contrast(galaxyColor(foreground), galaxyColor(background));
    expect(ratio, `${foreground} on ${background}`).toBeGreaterThanOrEqual(minimum);
  });

  it('keeps focus rings and panel borders visible', () => {
    expect(contrast(galaxyColor('--border-focus'), galaxyColor('--ink-canvas'))).toBeGreaterThanOrEqual(3);
    expect(contrast(galaxyColor('--border-focus'), galaxyColor('--ink-low'))).toBeGreaterThanOrEqual(3);
    expect(contrast(galaxyColor('--border-focus'), galaxyColor('--ink-mid'))).toBeGreaterThanOrEqual(3);
    expect(contrast(galaxyColor('--ink-hi'), galaxyColor('--ink-low'))).toBeGreaterThanOrEqual(1.3);
    expect(galaxyColor('--border-error')).not.toEqual(galaxyColor('--ink-low'));
  });

  it('avoids pure-white glare and a pure-black canvas', () => {
    expect(GALAXY_CSS.toLowerCase()).not.toContain('#ffffff');
    expect(GALAXY_CSS.toLowerCase()).not.toMatch(/rgba?\(\s*255\s*,\s*255\s*,\s*255/);
    expect(GALAXY_VALUES.get('--fg-bright')?.toLowerCase()).not.toBe('#ffffff');
    expect(GALAXY_VALUES.get('--ink-canvas')?.toLowerCase()).not.toBe('#000000');
    expect(luminance(galaxyColor('--fg-bright'))).toBeLessThan(0.95);
    expect(luminance(galaxyColor('--ink-canvas'))).toBeLessThan(0.02);
  });
});

describe('galaxy identity', () => {
  it('exposes one canonical spelling', () => {
    expect(GALAXY_THEME).toBe('galaxy');
    expect(isGalaxyTheme('galaxy')).toBe(true);
    expect(isGalaxyTheme(' Galaxy ')).toBe(true);
    expect(isGalaxyTheme('light')).toBe(false);
    expect(isGalaxyTheme('galaxy-dark')).toBe(false);
    expect(isGalaxyTheme('')).toBe(false);
  });
});