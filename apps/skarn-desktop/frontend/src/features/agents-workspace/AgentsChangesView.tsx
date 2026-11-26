import { useEffect, useState } from 'react';
import { agentsReadGitDiff, revealInExplorer, type BootstrapState } from '../../bridge/bridge';
import { attachAgentFile, openAgentFileInEditor } from './AgentsFilePreview';
import { Copy, ExternalLink, FolderOpen, Paperclip, RefreshCw } from '../../components/icons';

type Baseline = 'index' | 'working_tree';
type Diff = Awaited<ReturnType<typeof agentsReadGitDiff>>;
type Selection = { path: string; baseline: Baseline };
const normalize = (path: string) => path.replace(/\\/g, '/');

function renderDiffLines(diffText: string) {
  if (!diffText.includes('\n')) return diffText;
  const lines = diffText.split('\n');
  return lines.map((line, idx) => {
    let cls = 'agents-diff-line';
    if (line.startsWith('+') && !line.startsWith('+++')) cls += ' agents-diff-line--add';
    else if (line.startsWith('-') && !line.startsWith('---')) cls += ' agents-diff-line--del';
    else if (line.startsWith('@@')) cls += ' agents-diff-line--hunk';
    else if (line.startsWith('diff ') || line.startsWith('index ') || line.startsWith('---') || line.startsWith('+++')) cls += ' agents-diff-line--meta';
    return <span key={idx} className={cls}>{line}{idx < lines.length - 1 ? '\n' : ''}</span>;
  });
}

export function AgentsChangesView({ bootstrap }: { bootstrap: BootstrapState }) {
  const [selection, setSelection] = useState<Selection | null>(null);
  const [result, setResult] = useState<{ key: string; diff?: Diff; error?: string } | null>(null);
  const [revision, setRevision] = useState(0);
  const [actionError, setActionError] = useState<string | null>(null);
  const workspaceId = bootstrap.workspace.id;
  const key = `${workspaceId}\0${selection?.path}\0${selection?.baseline}\0${revision}`;
  const files = bootstrap.git.files ?? [...new Set([...bootstrap.git.changed_paths, ...bootstrap.git.staged_paths])].map((path) => ({
    path, staged: bootstrap.git.staged_paths.includes(path), worktree: bootstrap.git.changed_paths.includes(path),
  }));
  const staged = files.filter((file) => file.staged);
  const working = files.filter((file) => file.worktree);
  const dirty = bootstrap.buffers.filter((buffer) => buffer.dirty);
  const conflicts = bootstrap.git.conflicts ?? [];

  useEffect(() => { setSelection(null); setResult(null); setActionError(null); }, [workspaceId]);
  useEffect(() => {
    let current = true;
    if (!selection) return () => { current = false; };
    void agentsReadGitDiff(workspaceId, selection.path, selection.baseline).then((diff) => {
      if (current && diff.workspace_id === workspaceId && diff.path === selection.path && diff.baseline === selection.baseline) {
        setResult({ key, diff });
      }
    }).catch((error: unknown) => { if (current) setResult({ key, error: String(error) }); });
    return () => { current = false; };
  }, [workspaceId, selection, key, bootstrap.git]);

  const group = (title: string, entries: { path: string }[], baseline: Baseline) => <section aria-label={title}>
    <h3>{title} <span>{entries.length}</span></h3>
    {entries.length === 0 ? <p className="agents-context-muted">No files</p> : entries.map(({ path }) => {
      const fileName = path.split('/').pop() ?? path;
      const dirName = path.includes('/') ? path.split('/').slice(0, -1).join('/') : '';
      return (
        <button className="agents-change-row" type="button" key={path} title={path}
          aria-label={path}
          aria-pressed={selection?.path === path && selection.baseline === baseline}
          onClick={() => setSelection({ path, baseline })}>
          <span className={`agents-change-status-badge ${baseline === 'index' ? 'is-added' : 'is-modified'}`}>
            {baseline === 'index' ? 'A' : 'M'}
          </span>
          <span className="agents-change-filename">{fileName}</span>
          {dirName && <span className="agents-change-dir">{dirName}</span>}
        </button>
      );
    })}
  </section>;
  const reply = result?.key === key ? result : null;
  const selectedDirty = dirty.some((buffer) => normalize(buffer.path) === normalize(selection?.path ?? ''));

  return <div className="agents-changes-view">
    <p className="agents-context-muted">{bootstrap.git.branch ? `Branch: ${bootstrap.git.branch}` : 'No branch reported'}</p>
    {group('Staged · HEAD to index', staged, 'index')}
    {group('Working tree · index to disk', working, 'working_tree')}
    <section aria-label="Conflicts">
      <h3>Conflicts <span>{conflicts.length}</span></h3>
      {conflicts.length === 0 ? <p className="agents-context-muted">No conflicts</p> : conflicts.map((path) => (
        <button type="button" className="agents-change-row" key={path} aria-label={path} onClick={() => setSelection({ path, baseline: 'working_tree' })}>
          <span className="agents-change-status-badge is-conflict">!</span>
          <span className="agents-change-filename">{path.split('/').pop() ?? path}</span>
        </button>
      ))}
    </section>
    <section aria-label="Unsaved buffers">
      <h3>Unsaved buffers <span>{dirty.length}</span></h3>
      <p className="agents-context-muted">Unsaved text is not included in disk Git diffs. Use Files to preview buffer text.</p>
      {dirty.map((buffer) => <div key={buffer.buffer_id} className="agents-context-unsaved">{buffer.path}</div>)}
    </section>
    {selection && <section className="agents-diff-preview" aria-label={`Diff ${selection.path}`}>
      <header>
        <h3>{selection.path}</h3>
        <span className={`agents-preview-baseline-badge agents-preview-baseline-badge--${selection.baseline === 'index' ? 'index' : 'worktree'}`}>
          {selection.baseline === 'index' ? 'STAGED · HEAD → Index' : 'WORKING TREE · Index → Disk'}
        </span>
      </header>
      <p>{selection.baseline === 'index' ? 'Staged: HEAD to index' : 'Working tree: index to disk'}</p>
      {selectedDirty && <p className="agents-context-dirty">Unsaved editor edits are not included in this diff.</p>}
      <div className="agents-context-actions">
        <button type="button" onClick={() => {
          setActionError(null);
          void openAgentFileInEditor(selection.path, bootstrap).catch((error: unknown) => setActionError(String(error)));
        }}>
          <ExternalLink size={14} aria-hidden="true" />
          Open in Editor
        </button>
        <button type="button" onClick={() => attachAgentFile(selection.path, workspaceId)}>
          <Paperclip size={14} aria-hidden="true" />
          Attach
        </button>
        <button type="button" onClick={() => {
          setActionError(null);
          void Promise.resolve().then(() => revealInExplorer(selection.path, {
            core_instance_id: bootstrap.core_instance_id,
            workspace_id: workspaceId,
            session_id: null,
          })).catch((error: unknown) => setActionError(String(error)));
        }}>
          <FolderOpen size={14} aria-hidden="true" />
          Reveal in Explorer
        </button>
        <button type="button" onClick={() => {
          setActionError(null);
          void Promise.resolve().then(() => navigator.clipboard.writeText(selection.path))
            .catch((error: unknown) => setActionError(String(error)));
        }}>
          <Copy size={14} aria-hidden="true" />
          Copy path
        </button>
        <button type="button" onClick={() => setRevision((value) => value + 1)}>
          <RefreshCw size={14} aria-hidden="true" />
          Reload diff
        </button>
      </div>
      {actionError && <p role="alert">{actionError}</p>}
      {reply?.error ? <p role="alert">{reply.error}</p> : !reply?.diff ? <p role="status">Loading diff…</p>
        : <><pre tabIndex={0} className="agents-diff-pre">{reply.diff.unified_diff ? renderDiffLines(reply.diff.unified_diff) : 'No text diff available for this baseline.'}</pre>
          {reply.diff.truncated && <p role="status">Diff is partial: binary, oversized, or truncated at 256 KiB.</p>}</>}
    </section>}
  </div>;
}
