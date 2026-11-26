import { describe, expect, it } from 'vitest';
// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { readFileSync } from 'node:fs';
// @ts-ignore -- Node built-ins are available in Vitest's node runtime.
import { join } from 'node:path';

const css = readFileSync(
  join((globalThis as unknown as { process: { cwd(): string } }).process.cwd(), 'src/styles/dock.css'),
  'utf8',
);

describe('terminal layout', () => {
  it('constrains terminal flex children independently of xterm content', () => {
    for (const selector of ['terminal-body', 'terminal-view', 'terminal-xterm']) {
      const rule = css.match(new RegExp(`\\.${selector}\\s*\\{([^}]*)\\}`))?.[1];
      expect(rule).toContain('min-width: 0;');
      expect(rule).toContain('min-height: 0;');
      expect(rule).toContain('overflow: hidden;');
    }
  });
});
