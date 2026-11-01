import { useEffect, useState } from 'react';
import {
  agentsReadFilePreview, focusEditorWindow, openFile,
  type BootstrapState,
} from '../../bridge/bridge';
import { agentStore } from '../agent/agent-store';
import { hasPendingBufferEdits } from '../editor/pending-edits';
import { ExternalLink, Paperclip, RefreshCw } from '../../components/icons';

type Preview = Awaited<ReturnType<typeof agentsReadFilePreview>>;
const normalize = (path: string) => path.replace(/\\/g, '/');
const MAX_TEXT = 256 * 1024;

export function attachAgentFile(path: string, workspaceId: string): void {
  window.dispatchEvent(new CustomEvent('selora:attach-agent-file', {
    detail: { path, sessionId: agentStore.getSnapshot().activeSessionId, surface: 'agents', workspaceId },
  }));
}

export async function openAgentFileInEditor(path: string, bootstrap: BootstrapState): Promise<void> {
  const context = {
    core_instance_id: bootstrap.core_instance_id,
    workspace_id: bootstrap.workspace.id,
    session_id: null,
  };
  await openFile(path, context);
  await focusEditorWindow();
}

export function AgentsFilePreview({ bootstrap, path }: { bootstrap: BootstrapState; path: string | null }) {
  const workspaceId = bootstrap.workspace.id;
  const buffer = path ? bootstrap.buffers.find((candidate) => normalize(candidate.path) === normalize(path)) : undefined;
  const pending = buffer ? hasPendingBufferEdits(bootstrap, buffer.buffer_id) : false;
  const [result, setResult] = useState<{ key: string; preview?: Preview; error?: string } | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const key = `${workspaceId}\0${path ?? ''}\0${revision}`;

  useEffect(() => {
    let current = true;
    setActionError(null);
    if (!path || buffer) return () => { current = false; };
    void agentsReadFilePreview(workspaceId, path).then((preview) => {
      if (current && preview.workspace_id === workspaceId && normalize(preview.path) === normalize(path)) {
        setResult({ key, preview });
      }
    }).catch((error: unknown) => {
      if (current) setResult({ key, error: String(error) });
    });
    return () => { current = false; };
  }, [workspaceId, path, buffer?.buffer_id, key]);

  if (!path) return <div className="agents-context-empty">Select a file to preview. Main editor stays unchanged.</div>;
  const reply = result?.key === key ? result : null;
  const preview = reply?.preview;
  // Existing buffers, including dirty ones, always override asynchronous disk replies.
  let text = buffer?.text ?? preview?.text;
  let truncated = buffer ? new TextEncoder().encode(buffer.text).length > MAX_TEXT : preview?.truncated;
  if (buffer && truncated) {
    const bytes = new TextEncoder().encode(buffer.text).subarray(0, MAX_TEXT);
    text = new TextDecoder('utf-8', { fatal: false }).decode(bytes).replace(/\uFFFD$/, '');
  }
  const perform = (action: () => Promise<unknown>) => {
    setActionError(null);
    void action().catch((error: unknown) => setActionError(String(error)));
  };

  return <section className="agents-file-preview" aria-label={`Preview ${path}`}>
    <header>
      <strong title={path}>{path}</strong>
      <span className="agents-preview-source-badge">{buffer || preview?.source === 'buffer' ? 'Editor buffer' : 'Disk'}</span>
      {(buffer?.dirty || preview?.dirty) && <span className="agents-context-dirty">Unsaved</span>}
    </header>
    <div className="agents-context-actions">
      <button type="button" onClick={() => perform(() => openAgentFileInEditor(path, bootstrap))}>
        <ExternalLink size={14} aria-hidden="true" />
        Open in Editor
      </button>
      <button type="button" onClick={() => attachAgentFile(path, workspaceId)}>
        <Paperclip size={14} aria-hidden="true" />
        Attach
      </button>
      <button type="button" onClick={() => setRevision((value) => value + 1)}>
        <RefreshCw size={14} aria-hidden="true" />
        Reload preview
      </button>
    </div>
    {actionError && <p role="alert">{actionError}</p>}
    {pending ? <p role="status">Editor changes syncing. Preview waits for authoritative buffer text.</p>
      : reply?.error && !buffer ? <p role="alert">{reply.error}</p>
      : preview?.binary && !buffer ? <p>Binary or non-UTF-8 file. Open in Editor to inspect.</p>
      : text == null ? <p role="status">Loading preview…</p>
      : <><pre tabIndex={0} className="agents-code-viewer">{text}</pre>{truncated && <p role="status">Preview truncated at 256 KiB.</p>}</>}
  </section>;
}
