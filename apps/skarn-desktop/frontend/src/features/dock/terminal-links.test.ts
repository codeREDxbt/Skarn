import { describe, expect, it } from 'vitest';
import { findTerminalLinks, resolveWorkspacePath } from './terminal-links';

describe('terminal links', () => {
  it('parses Windows paths with line and column', () => {
    expect(findTerminalLinks('error C:\\work\\src\\main.rs:12:7: nope')[0]).toMatchObject({
      path: 'C:\\work\\src\\main.rs', line: 12, column: 7,
    });
  });

  it('parses relative paths and ignores URLs', () => {
    expect(findTerminalLinks('src/main.ts:4:2 https://example.test/a')).toHaveLength(1);
    expect(findTerminalLinks('src/main.ts:4:2')[0].path).toBe('src/main.ts');
  });

  it('keeps navigation inside workspace', () => {
    expect(resolveWorkspacePath('src/main.ts', 'C:\\work')).toBe('src/main.ts');
    expect(resolveWorkspacePath('../secrets.txt', 'C:\\work')).toBeNull();
    expect(resolveWorkspacePath('C:\\other\\x.ts', 'C:\\work')).toBeNull();
  });

  it('handles POSIX and UNC roots without accepting escapes or device paths', () => {
    expect(resolveWorkspacePath('/work/src/a.ts', '/work')).toBe('src/a.ts');
    expect(resolveWorkspacePath('/Work/a.ts', '/work')).toBeNull();
    expect(resolveWorkspacePath('../work-other/a.ts', '/work')).toBeNull();
    expect(resolveWorkspacePath('src/a.ts', '\\\\server\\share\\work')).toBe('src/a.ts');
    expect(resolveWorkspacePath('\\\\?\\C:\\work\\a.ts', 'C:\\work')).toBeNull();
    expect(resolveWorkspacePath('C:secret', 'C:\\work')).toBeNull();
    expect(resolveWorkspacePath('file:///work/a.ts', '/work')).toBeNull();
  });

  it('parses line-only locations and POSIX absolute paths', () => {
    expect(findTerminalLinks('/work/main.rs:42:')[0]).toMatchObject({ path: '/work/main.rs', line: 42 });
  });
});
