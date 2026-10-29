import { describe, expect, it } from 'vitest';
import {
  MAX_EDITOR_DOCUMENT_CHARS,
  MAX_EDITOR_LINE_CHARS,
  documentGuard,
  largeDocumentNotice,
  languageFor,
} from './language-loaders';

describe('large-file document guard', () => {
  it('passes a normal source through', () => {
    const guard = documentGuard('fn main() {\n    println!("hi");\n}\n');
    expect(guard.large).toBe(false);
    expect(guard.reason).toBeNull();
    expect(guard.longestLineChars).toBeLessThanOrEqual('    println!("hi");'.length);
    expect(largeDocumentNotice(guard)).toBeNull();
  });

  it('guards documents over the 2 MB editor limit', () => {
    const guard = documentGuard('a'.repeat(MAX_EDITOR_DOCUMENT_CHARS + 1));
    expect(guard.large).toBe(true);
    expect(guard.reason).toBe('size');
    expect(guard.chars).toBe(MAX_EDITOR_DOCUMENT_CHARS + 1);
    expect(largeDocumentNotice(guard)).toContain('too large');
  });

  it('guards a minified line below the size limit', () => {
    const minified = `var a=${'x'.repeat(MAX_EDITOR_LINE_CHARS)};`;
    const guard = documentGuard(minified);
    expect(guard.large).toBe(true);
    expect(guard.reason).toBe('minified');
    expect(largeDocumentNotice(guard)).toContain('very long line');
  });

  it('allows a line exactly at the minified threshold', () => {
    const edge = 'x'.repeat(MAX_EDITOR_LINE_CHARS);
    expect(documentGuard(edge).large).toBe(false);
    expect(documentGuard(`${edge}y`).large).toBe(true);
  });

  it('keeps multi-line files with normal lines unguarded', () => {
    const text = `${'a'.repeat(MAX_EDITOR_LINE_CHARS)}\n${'b'.repeat(200)}\n`;
    const guard = documentGuard(text);
    expect(guard.large).toBe(false);
    expect(guard.longestLineChars).toBe(MAX_EDITOR_LINE_CHARS);
  });

  it('handles the empty document', () => {
    expect(documentGuard('')).toEqual({
      large: false,
      reason: null,
      chars: 0,
      longestLineChars: 0,
    });
  });
});

describe('language selection', () => {
  it('still maps extensions for the ordinary path', () => {
    expect(languageFor('src/main.rs')).toBe('rust');
    expect(languageFor('build/out.js')).toBe('javascript');
    expect(languageFor('README.md')).toBe('markdown');
    expect(languageFor('LICENSE')).toBe('plain');
  });
});