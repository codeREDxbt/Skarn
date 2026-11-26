import type { Extension } from '@codemirror/state';

export type LanguageId =
  | 'plain'
  | 'rust'
  | 'javascript'
  | 'jsx'
  | 'typescript'
  | 'tsx'
  | 'css'
  | 'json'
  | 'markdown';

type ImportedLanguageId = Exclude<LanguageId, 'plain'>;

/**
 * Documents larger than this are opened in a read-only, plain-text mode.
 * Mirrors the desktop workspace's 2 MiB editor limit
 * (`workspace.rs::MAX_FILE_BYTES`), so the frontend guard also covers a
 * buffer that reached the editor through another path (history restore,
 * agent write) and protects the WebView from a huge CodeMirror state.
 */
export const MAX_EDITOR_DOCUMENT_CHARS = 2 * 1024 * 1024;

/**
 * A single line longer than this is treated as minified/generated. Syntax
 * parsing and linting scan line content; one multi-MB line is what froze the
 * pane even below the document cap.
 */
export const MAX_EDITOR_LINE_CHARS = 10_000;

export type DocumentGuard = {
  /** `true` when highlighting, linting and editing must be off. */
  large: boolean;
  /** Why the document is guarded, `null` for a normal file. */
  reason: 'size' | 'minified' | null;
  /** UTF-16 code units in the document. */
  chars: number;
  /** Longest line, in UTF-16 code units (`0` when the size cap short-circuits). */
  longestLineChars: number;
};

/**
 * Classify a buffer for the large-file fallback. Pure and bounded: `size` is
 * decided by `length`, and the line scan stops as soon as it exceeds
 * {@link MAX_EDITOR_LINE_CHARS} (or at the document cap).
 */
export function documentGuard(text: string): DocumentGuard {
  const chars = text.length;
  if (chars > MAX_EDITOR_DOCUMENT_CHARS) {
    return { large: true, reason: 'size', chars, longestLineChars: 0 };
  }

  let longestLineChars = 0;
  let lineStart = 0;
  for (;;) {
    const newline = text.indexOf('\n', lineStart);
    const lineEnd = newline === -1 ? chars : newline;
    const length = lineEnd - lineStart;
    if (length > longestLineChars) {
      longestLineChars = length;
      if (longestLineChars > MAX_EDITOR_LINE_CHARS) {
        return { large: true, reason: 'minified', chars, longestLineChars };
      }
    }
    if (newline === -1) break;
    lineStart = newline + 1;
  }

  return { large: false, reason: null, chars, longestLineChars };
}

/** A short, user-facing explanation for the read-only fallback banner. */
export function largeDocumentNotice(guard: DocumentGuard): string | null {
  if (!guard.large) return null;
  if (guard.reason === 'minified') {
    return 'This file has a very long line (minified or generated). Opened read-only without syntax highlighting so the editor stays responsive.';
  }
  return 'This file is too large for the editor (over 2 MB). Opened read-only without syntax highlighting so the editor stays responsive.';
}

function extensionOf(path: string): string {
  const filename = path.split(/[\\/]/).pop() ?? path;
  const dot = filename.lastIndexOf('.');
  return dot === -1 ? '' : filename.slice(dot + 1).toLowerCase();
}

export function languageFor(path: string): LanguageId {
  switch (extensionOf(path)) {
    case 'rs':
      return 'rust';
    case 'js':
      return 'javascript';
    case 'jsx':
      return 'jsx';
    case 'ts':
      return 'typescript';
    case 'tsx':
      return 'tsx';
    case 'css':
      return 'css';
    case 'json':
      return 'json';
    case 'md':
    case 'markdown':
      return 'markdown';
    default:
      return 'plain';
  }
}

const languageLoaders: Record<ImportedLanguageId, () => Promise<Extension>> = {
  rust: async () => (await import('@codemirror/lang-rust')).rust(),
  javascript: async () => (await import('@codemirror/lang-javascript')).javascript(),
  jsx: async () => (await import('@codemirror/lang-javascript')).javascript({ jsx: true }),
  typescript: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ typescript: true }),
  tsx: async () =>
    (await import('@codemirror/lang-javascript')).javascript({ typescript: true, jsx: true }),
  css: async () => (await import('@codemirror/lang-css')).css(),
  json: async () => (await import('@codemirror/lang-json')).json(),
  markdown: async () => (await import('@codemirror/lang-markdown')).markdown(),
};

export async function loadLanguage(id: LanguageId): Promise<Extension> {
  if (id === 'plain') return [];
  return languageLoaders[id]();
}
