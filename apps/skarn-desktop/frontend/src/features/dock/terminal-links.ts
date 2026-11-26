export type TerminalLink = {
  path: string;
  line: number;
  column: number | null;
  start: number;
  length: number;
};

const LOCATION_RE = /(?:^|[\s([{"'`])((?:[A-Za-z]:[\\/]|[\\/]|\.\.?[\\/]|[A-Za-z0-9_@.$-]+[\\/])[^\s)\]}>,;'"`]*)(?=\s|$|[)\]}>,;'"`])/g;

/** Parse file paths emitted by common compilers, including Windows locations. */
export function findTerminalLinks(text: string): TerminalLink[] {
  const links: TerminalLink[] = [];
  for (const match of text.matchAll(LOCATION_RE)) {
    const token = match[1];
    if (!token || /^(?:https?|file):\/\//i.test(token)) continue;
    const location = token.match(/^(.*?):(\d+)(?::(\d+))?:?$/);
    const path = location?.[1] ?? token;
    if (!path) continue;
    const start = (match.index ?? 0) + match[0].indexOf(path);
    links.push({
      path,
      line: location?.[2] ? Number(location[2]) : 1,
      column: location?.[3] ? Number(location[3]) : null,
      start,
      length: token.length,
    });
  }
  return links;
}

/** Apply lexical containment; the workspace backend must also check symlinks. */
export function resolveWorkspacePath(path: string, workspaceRoot: string | null): string | null {
  if (!workspaceRoot || !path || /[\x00-\x1f\x7f]/.test(path)) return null;
  const normalize = (value: string): string | null => {
    value = value.replace(/\\/g, '/');
    if (/^\/\/[?.]\//.test(value)) return null;
    const prefix = value.startsWith('//') ? '//' : value.startsWith('/') ? '/' : /^[A-Za-z]:\//.test(value) ? value.slice(0, 3) : '';
    if (!prefix) return null;
    const parts: string[] = [];
    for (const part of value.slice(prefix.length).split('/')) {
      if (!part || part === '.') continue;
      if (part === '..') { if (!parts.length) return null; parts.pop(); }
      else if (part.includes(':')) return null;
      else parts.push(part);
    }
    return prefix + parts.join('/');
  };
  const root = normalize(workspaceRoot);
  if (!root || /^(?:[A-Za-z]:[^/\\]|[A-Za-z][\w+.-]*:)/.test(path.replace(/\\/g, '/')) && !/^[A-Za-z]:[\\/]/.test(path)) return null;
  const candidate = path.replace(/\\/g, '/');
  const resolved = normalize(candidate.startsWith('/') || /^[A-Za-z]:\//.test(candidate) ? candidate : `${root}/${candidate}`);
  if (!resolved) return null;
  const windows = /^[A-Za-z]:\//.test(root) || root.startsWith('//');
  const compare = (value: string) => windows ? value.toLowerCase() : value;
  const base = compare(root.replace(/\/$/, ''));
  const target = compare(resolved);
  if (target !== base && !target.startsWith(`${base}/`)) return null;
  const relative = resolved.slice(root.replace(/\/$/, '').length).replace(/^\//, '');
  return relative || null;
}
