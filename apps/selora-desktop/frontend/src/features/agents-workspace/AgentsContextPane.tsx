import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { refreshTree, revealInExplorer, type BootstrapState, type FileTreeNode } from '../../bridge/bridge';
import {
  ChevronDown,
  ChevronRight,
  Copy,
  FolderOpen,
  Paperclip,
  RefreshCw,
} from '../../components/icons';
import { FileTypeIcon } from '../sidebar/FileTreeView';
import { AgentsFilePreview, attachAgentFile } from './AgentsFilePreview';
import { AgentsChangesView } from './AgentsChangesView';
import './agents-context.css';

type Tab = 'files' | 'changes';
const TAB_KEY = 'agents:last-context-tab';
function initialTab(): Tab {
  try { return localStorage.getItem(TAB_KEY) === 'changes' ? 'changes' : 'files'; } catch { return 'files'; }
}

export function AgentsContextPane({ bootstrap }: { bootstrap: BootstrapState }) {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set([bootstrap.file_tree.root?.path ?? '']));
  const [selected, setSelected] = useState<{ workspaceId: string; path: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  const workspaceId = bootstrap.workspace.id;
  const selectedPath = selected?.workspaceId === workspaceId ? selected.path : null;
  const context = { core_instance_id: bootstrap.core_instance_id, workspace_id: workspaceId, session_id: null };

  useEffect(() => {
    const listener = (event: Event) => {
      const requested = (event as CustomEvent<{ tab?: unknown }>).detail?.tab;
      if (requested === 'files' || requested === 'changes') setTab(requested);
    };
    window.addEventListener('selora:agents-context-tab', listener);
    return () => window.removeEventListener('selora:agents-context-tab', listener);
  }, []);

  // Listen to attachment preview from composer
  useEffect(() => {
    const previewListener = (event: Event) => {
      const detail = (event as CustomEvent<{ path?: string }>).detail;
      if (detail?.path) {
        setTab('files');
        setSelected({ workspaceId, path: detail.path });
        const parts = detail.path.split(/[\\/]/);
        setExpanded((prev) => {
          const next = new Set(prev);
          let curr = '';
          for (let i = 0; i < parts.length - 1; i++) {
            curr = curr ? `${curr}/${parts[i]}` : parts[i];
            next.add(curr);
          }
          return next;
        });
      }
    };
    window.addEventListener('selora:preview-agent-file', previewListener);
    return () => window.removeEventListener('selora:preview-agent-file', previewListener);
  }, [workspaceId]);

  useEffect(() => { try { localStorage.setItem(TAB_KEY, tab); } catch { /* Storage may be disabled. */ } }, [tab]);
  useEffect(() => { setExpanded(new Set([bootstrap.file_tree.root?.path ?? ''])); setSelected(null); setError(null); }, [workspaceId, bootstrap.file_tree.root?.path]);

  const perform = (action: () => Promise<unknown>) => {
    setError(null);
    try {
      void Promise.resolve(action()).catch((cause: unknown) => setError(String(cause)));
    } catch (cause) {
      setError(String(cause));
    }
  };
  const toggle = (path: string, force?: boolean) => setExpanded((previous) => {
    const next = new Set(previous);
    if (force ?? !next.has(path)) next.add(path); else next.delete(path);
    return next;
  });
  const onTreeKey = (event: KeyboardEvent<HTMLButtonElement>, node: FileTreeNode) => {
    const rows = Array.from(treeRef.current?.querySelectorAll<HTMLButtonElement>('[role="treeitem"]') ?? []);
    const index = rows.indexOf(event.currentTarget);
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      const target = event.key === 'Home' ? 0 : event.key === 'End' ? rows.length - 1 : index + (event.key === 'ArrowDown' ? 1 : -1);
      rows[Math.max(0, Math.min(rows.length - 1, target))]?.focus();
    } else if (event.key === 'ArrowRight' && node.kind === 'directory') {
      event.preventDefault();
      if (!expanded.has(node.path)) toggle(node.path, true); else rows[index + 1]?.focus();
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      if (node.kind === 'directory' && expanded.has(node.path)) toggle(node.path, false);
      else {
        const parent = node.path.split('/').slice(0, -1).join('/') || bootstrap.file_tree.root?.path;
        rows.find((row) => row.dataset.path === parent)?.focus();
      }
    }
  };
  const renderNode = (node: FileTreeNode, depth: number) => {
    const directory = node.kind === 'directory';
    const open = expanded.has(node.path);
    return <div key={node.path} role="none">
      <button type="button" role="treeitem" data-path={node.path}
        aria-level={depth + 1} aria-expanded={directory ? open : undefined} aria-selected={!directory && selectedPath === node.path}
        className="agents-tree-row" style={{ paddingInlineStart: `${depth * 14 + 6}px` }} title={node.path || node.name}
        onKeyDown={(event) => onTreeKey(event, node)}
        onClick={() => directory ? toggle(node.path) : setSelected({ workspaceId, path: node.path })}>
        {directory ? (
          <>
            <span className="agents-tree-chevron" aria-hidden="true" style={{ transform: open ? 'rotate(90deg)' : 'none' }}>
              <ChevronRight size={13} />
            </span>
            <FileTypeIcon name={node.name} isFolder isOpen={open} />
          </>
        ) : (
          <>
            <span className="agents-tree-spacer" aria-hidden="true" />
            <FileTypeIcon name={node.name} />
          </>
        )}
        <span>{node.name}</span>
      </button>
      {directory && open && <div role="group">{node.children.map((child) => renderNode(child, depth + 1))}</div>}
    </div>;
  };

  const totalChanges = (bootstrap.git.files?.length ?? 0) || (bootstrap.git.changed_paths.length + bootstrap.git.staged_paths.length);

  return <aside className="agents-context-pane" aria-label="Agent workspace context">
    <div className="agents-context-tabs" role="tablist" aria-label="Workspace context">
      <button id="agents-files-tab" role="tab" aria-selected={tab === 'files'} aria-controls="agents-context-content" onClick={() => setTab('files')}>Files</button>
      <button id="agents-changes-tab" role="tab" aria-selected={tab === 'changes'} aria-controls="agents-context-content" onClick={() => setTab('changes')}>
        Changes
        {totalChanges > 0 && <span className="agents-context-tab-counter" aria-hidden="true">{totalChanges}</span>}
      </button>
      <button type="button" title="Refresh workspace tree" aria-label="Refresh workspace tree"
        onClick={() => perform(() => refreshTree(context))}><RefreshCw size={14} /></button>
    </div>
    {error && <p role="alert">{error}</p>}
    <div id="agents-context-content" role="tabpanel" aria-labelledby={tab === 'files' ? 'agents-files-tab' : 'agents-changes-tab'}>
      {!bootstrap.workspace.root ? <div className="agents-context-empty">Open a workspace in the Editor window.</div>
        : tab === 'changes' ? <AgentsChangesView key={workspaceId} bootstrap={bootstrap} />
        : <>
          <div ref={treeRef} className="agents-context-tree" role="tree" aria-label="Workspace files">
            {bootstrap.file_tree.root ? renderNode(bootstrap.file_tree.root, 0) : <p>No file tree available. Refresh to retry.</p>}
          </div>
          {selectedPath && <div className="agents-context-actions">
            <button type="button" onClick={() => perform(() => revealInExplorer(selectedPath, context))}>
              <FolderOpen size={13} aria-hidden="true" />
              Reveal in Explorer
            </button>
            <button type="button" onClick={() => perform(() => navigator.clipboard.writeText(selectedPath))}>
              <Copy size={13} aria-hidden="true" />
              Copy path
            </button>
            <button type="button" onClick={() => attachAgentFile(selectedPath, workspaceId)}>
              <Paperclip size={13} aria-hidden="true" />
              Attach file
            </button>
          </div>}
          <AgentsFilePreview key={workspaceId} bootstrap={bootstrap} path={selectedPath} />
        </>}
    </div>
  </aside>;
}
