import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BootstrapState } from '../../bridge/bridge';

const mocked = vi.hoisted(() => ({
  read: vi.fn(), diff: vi.fn(), open: vi.fn(async () => undefined), focus: vi.fn(async () => undefined),
  refresh: vi.fn(async () => undefined), reveal: vi.fn(async () => undefined), pending: false,
}));
vi.mock('../../bridge/bridge', () => ({
  agentsReadFilePreview: mocked.read, agentsReadGitDiff: mocked.diff, openFile: mocked.open,
  focusEditorWindow: mocked.focus, refreshTree: mocked.refresh, revealInExplorer: mocked.reveal,
}));
vi.mock('../agent/agent-store', () => ({ agentStore: { getSnapshot: () => ({ activeSessionId: 'session' }) } }));
vi.mock('../editor/pending-edits', () => ({ hasPendingBufferEdits: () => mocked.pending }));
vi.mock('../sidebar/FileTreeView', () => ({ FileTypeIcon: () => null }));
import { AgentsContextPane } from './AgentsContextPane';
import { AgentsFilePreview } from './AgentsFilePreview';
import { AgentsChangesView } from './AgentsChangesView';

function bootstrap(id = 'workspace'): BootstrapState {
  return {
    core_instance_id: 'core', workspace: { id, root: 'C:/project', display_name: 'project' },
    buffers: [], file_tree: { root: { path: 'project', name: 'project', kind: 'directory', children: [
      { path: 'a.ts', name: 'a.ts', kind: 'file', children: [] },
      { path: 'b.ts', name: 'b.ts', kind: 'file', children: [] },
    ] } },
    git: { branch: 'main', files: [{ path: 'a.ts', staged: true, worktree: true }], changed_paths: ['a.ts'], staged_paths: ['a.ts'], conflicts: ['b.ts'] },
  } as unknown as BootstrapState;
}
const preview = (workspaceId: string, path: string, text: string) => ({
  workspace_id: workspaceId, path, text, source: 'disk', dirty: false, version: null,
  byte_length: text.length, truncated: false, binary: false,
});

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocked.pending = false;
  mocked.read.mockImplementation(async (workspaceId: string, path: string) => preview(workspaceId, path, `text ${path}`));
  mocked.diff.mockImplementation(async (workspaceId: string, path: string, baseline: string) => ({
    workspace_id: workspaceId, path, baseline, unified_diff: `diff ${baseline}`, truncated: false,
  }));
});
afterEach(cleanup);

describe('Agents workspace context', () => {
  it('previews real tree files without opening; only explicit open focuses main editor', async () => {
    render(<AgentsContextPane bootstrap={bootstrap()} />);
    fireEvent.click(screen.getByRole('treeitem', { name: 'a.ts' }));
    await screen.findByText('text a.ts');
    expect(mocked.open).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Open in Editor' }));
    await waitFor(() => expect(mocked.focus).toHaveBeenCalledOnce());
    expect(mocked.open).toHaveBeenCalledWith('a.ts', { core_instance_id: 'core', workspace_id: 'workspace', session_id: null });
    expect(mocked.open.mock.invocationCallOrder[0]).toBeLessThan(mocked.focus.mock.invocationCallOrder[0]);
  });

  it('uses authoritative dirty buffer text and hides it while local edits await acknowledgement', () => {
    const state = bootstrap();
    state.buffers = [{ buffer_id: 'buffer', path: 'a.ts', text: 'unsaved authoritative text', dirty: true, version: 2, language: 'typescript', selection: null }];
    const { rerender } = render(<AgentsFilePreview bootstrap={state} path="a.ts" />);
    expect(screen.getByText('unsaved authoritative text')).toBeTruthy();
    expect(mocked.read).not.toHaveBeenCalled();
    mocked.pending = true;
    rerender(<AgentsFilePreview bootstrap={state} path="a.ts" />);
    expect(screen.queryByText('unsaved authoritative text')).toBeNull();
    expect(screen.getByRole('status').textContent).toContain('syncing');
  });

  it('discards stale path and workspace replies', async () => {
    let resolveFirst!: (value: ReturnType<typeof preview>) => void;
    mocked.read.mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }));
    const { rerender } = render(<AgentsFilePreview bootstrap={bootstrap()} path="a.ts" />);
    rerender(<AgentsFilePreview bootstrap={bootstrap('second')} path="b.ts" />);
    await screen.findByText('text b.ts');
    await act(async () => resolveFirst(preview('workspace', 'a.ts', 'stale text')));
    expect(screen.queryByText('stale text')).toBeNull();
  });

  it('persists requested tab and reads independent staged and working tree baselines', async () => {
    render(<AgentsContextPane bootstrap={bootstrap()} />);
    act(() => window.dispatchEvent(new CustomEvent('skarn:agents-context-tab', { detail: { tab: 'changes' } })));
    expect(localStorage.getItem('skarn:agents-last-context-tab')).toBe('changes');
    const rows = screen.getAllByRole('button', { name: 'a.ts' });
    fireEvent.click(rows[0]);
    await screen.findByText('diff index');
    expect(mocked.diff).toHaveBeenLastCalledWith('workspace', 'a.ts', 'index');
    fireEvent.click(rows[1]);
    await screen.findByText('diff working_tree');
    expect(mocked.diff).toHaveBeenLastCalledWith('workspace', 'a.ts', 'working_tree');
    expect(screen.getByRole('region', { name: 'Conflicts' }).textContent).toContain('b.ts');
    expect(mocked.open).not.toHaveBeenCalled();
  });

  it('attaches file with current session and workspace identity', async () => {
    const listener = vi.fn();
    window.addEventListener('skarn:attach-agent-file', listener);
    render(<AgentsFilePreview bootstrap={bootstrap()} path="a.ts" />);
    fireEvent.click(screen.getByRole('button', { name: 'Attach' }));
    expect((listener.mock.calls[0][0] as CustomEvent).detail).toEqual({ path: 'a.ts', sessionId: 'session', surface: 'agents', workspaceId: 'workspace' });
    window.removeEventListener('skarn:attach-agent-file', listener);
  });

  it('supports keyboard tree navigation, refresh, reveal, and clipboard', async () => {
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    render(<AgentsContextPane bootstrap={bootstrap()} />);
    const root = screen.getByRole('treeitem', { name: 'project' });
    fireEvent.keyDown(root, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('treeitem', { name: 'a.ts' }));
    fireEvent.click(screen.getByRole('treeitem', { name: 'a.ts' }));
    fireEvent.click(screen.getByRole('button', { name: 'Refresh workspace tree' }));
    fireEvent.click(screen.getByRole('button', { name: 'Reveal in Explorer' }));
    fireEvent.click(screen.getByRole('button', { name: 'Copy path' }));
    expect(mocked.refresh).toHaveBeenCalledOnce();
    expect(mocked.reveal).toHaveBeenCalledWith('a.ts', expect.objectContaining({ workspace_id: 'workspace' }));
    expect(writeText).toHaveBeenCalledWith('a.ts');
    await screen.findByText('text a.ts');
  });

  it('reports native failures and partial diffs truthfully', async () => {
    mocked.diff.mockRejectedValueOnce('Repository unavailable');
    render(<AgentsChangesView bootstrap={bootstrap()} />);
    fireEvent.click(screen.getAllByRole('button', { name: 'a.ts' })[0]);
    expect(await screen.findByRole('alert')).toBeTruthy();
    mocked.diff.mockResolvedValueOnce({ workspace_id: 'workspace', path: 'a.ts', baseline: 'index', unified_diff: 'partial', truncated: true });
    fireEvent.click(screen.getByRole('button', { name: 'Reload diff' }));
    await screen.findByText('partial');
    expect(screen.getByRole('status').textContent).toContain('partial');
  });

  it('reports binary previews and does not focus main when opening fails', async () => {
    mocked.read.mockResolvedValueOnce({ ...preview('workspace', 'a.ts', ''), text: null, binary: true });
    mocked.open.mockRejectedValueOnce(new Error('Workspace changed'));
    render(<AgentsFilePreview bootstrap={bootstrap()} path="a.ts" />);
    await screen.findByText('Binary or non-UTF-8 file. Open in Editor to inspect.');
    fireEvent.click(screen.getByRole('button', { name: 'Open in Editor' }));
    expect((await screen.findByRole('alert')).textContent).toContain('Workspace changed');
    expect(mocked.focus).not.toHaveBeenCalled();
  });

  it('discards late diffs after selecting a different baseline', async () => {
    let resolveFirst!: (value: unknown) => void;
    mocked.diff.mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }));
    render(<AgentsChangesView bootstrap={bootstrap()} />);
    const rows = screen.getAllByRole('button', { name: 'a.ts' });
    fireEvent.click(rows[0]);
    fireEvent.click(rows[1]);
    await screen.findByText('diff working_tree');
    await act(async () => resolveFirst({ workspace_id: 'workspace', path: 'a.ts', baseline: 'index', unified_diff: 'stale diff', truncated: false }));
    expect(screen.queryByText('stale diff')).toBeNull();
  });
});
