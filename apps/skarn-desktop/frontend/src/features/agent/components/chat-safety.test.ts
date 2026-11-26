import { describe, expect, it } from 'vitest';
import { diagnosticClipboardText, safeWorkspaceRelativePath } from './chat-safety';

describe('chat safety', () => {
  it('accepts files below workspace root and rejects outside or traversing paths', () => {
    expect(safeWorkspaceRelativePath('C:/repo', 'C:/repo/src/main.rs')).toBe('src/main.rs');
    expect(safeWorkspaceRelativePath('C:/repo', 'C:/other/secret.env')).toBeNull();
    expect(safeWorkspaceRelativePath('C:/repo', 'C:/repo/../secret.env')).toBeNull();
    expect(safeWorkspaceRelativePath('C:/repo', '../secret.env')).toBeNull();
  });

  it('handles root boundaries, Windows paths, and unsafe relative paths', () => {
    expect(safeWorkspaceRelativePath('C:/', 'c:/src/main.ts')).toBe('src/main.ts');
    expect(safeWorkspaceRelativePath('C:/repo', 'C:/repository/file')).toBeNull();
    expect(safeWorkspaceRelativePath('C:/repo', 'C:secret')).toBeNull();
    expect(safeWorkspaceRelativePath('C:/repo', 'src/./file')).toBeNull();
    expect(safeWorkspaceRelativePath('C:/repo', 'src/evil\nfile')).toBeNull();
    expect(safeWorkspaceRelativePath('/repo', '/repo/../secret')).toBeNull();
    expect(safeWorkspaceRelativePath('//server/share', '//SERVER/share/a.ts')).toBe('a.ts');
    expect(safeWorkspaceRelativePath(null, 'src/main.ts')).toBeNull();
  });

  it('formats diagnostics with relative paths and one-based locations', () => {
    expect(
      diagnosticClipboardText('C:/repo', {
        path: 'C:/repo/src/main.rs',
        line: 2,
        column: 4,
        severity: 'error',
        message: 'cannot find value `x`',
      }),
    ).toBe('src/main.rs:3:5 — error: cannot find value `x`');
  });
});
