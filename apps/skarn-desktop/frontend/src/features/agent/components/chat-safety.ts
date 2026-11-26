import type { Diagnostic } from '../../../bridge/bridge';

/** Return a workspace-relative path, or null when path cannot be proven safe. */
export function safeWorkspaceRelativePath(root: string | null, path: string): string | null {
  if (!root || !path.trim() || /[\x00-\x1f\x7f]/.test(root + path)) return null;
  const normalize = (value: string) => value.trim().replace(/\\/g, '/');
  const rawRoot = normalize(root);
  const normalizedRoot = /^[A-Za-z]:\/$/.test(rawRoot) ? rawRoot : rawRoot.replace(/\/+$/, '') || '/';
  const normalizedPath = normalize(path);
  const absolute = (value: string) => value.startsWith('/') || /^[A-Za-z]:\//.test(value);
  if (!absolute(normalizedRoot) || /^[A-Za-z]:[^/]/.test(normalizedPath)) return null;
  if (normalizedRoot.split('/').some((part) => part === '.' || part === '..')) return null;
  let relative = normalizedPath;
  if (absolute(normalizedPath)) {
    const windows = /^[A-Za-z]:\//.test(normalizedRoot) || normalizedRoot.startsWith('//');
    const rootKey = windows ? normalizedRoot.toLowerCase() : normalizedRoot;
    const pathKey = windows ? normalizedPath.toLowerCase() : normalizedPath;
    const prefix = rootKey.endsWith('/') ? rootKey : `${rootKey}/`;
    if (!pathKey.startsWith(prefix)) return null;
    relative = normalizedPath.slice(prefix.length);
  }
  const parts = relative.split('/');
  if (parts.some((part) => !part || part === '.' || part === '..' || part.includes(':'))) return null;
  return parts.join('/');
}

export function diagnosticClipboardText(root: string | null, diagnostic: Diagnostic): string {
  const relative = safeWorkspaceRelativePath(root, diagnostic.path);
  const location = relative ? `${relative}:${diagnostic.line + 1}:${diagnostic.column + 1} — ` : '';
  return `${location}${diagnostic.severity}: ${diagnostic.message}`;
}
