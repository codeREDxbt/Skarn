import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { AppRuntime } from '../../app/providers';
import { currentEnvelopeContext, getAppSnapshot, retryBootstrap, subscribeApp } from '../../app/app-store';
import { agentsWindowReady, focusEditorWindow, openExternal, openFolderAt, pickFolder, subscribeAgentsNavigation, type AgentsNavigationTarget } from '../../bridge/bridge';
import { CommandPalette } from '../../components/CommandPalette';
import { ToastViewport } from '../../components/ToastViewport';
import { agentStore } from '../agent/agent-store';
import { buildCommandList, runCommandAction, type AgentsCommand, type CommandId } from '../keymap/commands';
import { useShortcuts } from '../keymap/use-shortcuts';
import { toastStore } from '../toasts/toast-store';
import { isNativeFrontend } from '../../app/surface';
import { AgentsWorkspaceHeader } from './AgentsWorkspaceHeader';
import { AgentsWorkspaceShell } from './AgentsWorkspaceShell';
import { AgentsNavigationRail } from './AgentsNavigationRail';
import { AGENTS_LAYOUT_BOUNDS, agentsWorkspaceStore, type AgentsWorkspacePage, type AgentsWorkspaceVisit } from './agents-workspace-store';

const AgentConversationSurface = lazy(() => import('../agent/components/AgentConversationSurface').then((module) => ({ default: module.AgentConversationSurface })));
const AgentsContextPane = lazy(async () => {
  const { AgentsContextPane: Pane } = await import('./AgentsContextPane');
  return { default: function ContextSlot({ bootstrap, requestedTab }: {
    bootstrap: NonNullable<ReturnType<typeof getAppSnapshot>['bootstrap']>;
    requestedTab: { tab: 'files' | 'changes' } | null;
  }) {
    useEffect(() => {
      if (requestedTab) window.dispatchEvent(new CustomEvent('selora:agents-context-tab', { detail: requestedTab }));
    }, [requestedTab]);
    return <Pane bootstrap={bootstrap} />;
  } };
});
const AgentsAutomationsView = lazy(() => import('./AgentsAutomationsView').then((module) => ({ default: module.AgentsAutomationsView })));
const AgentsCustomizationsView = lazy(() => import('./AgentsCustomizationsView').then((module) => ({ default: module.AgentsCustomizationsView })));
const ConnectModal = lazy(() => import('../auth/ConnectModal').then((module) => ({ default: module.ConnectModal })));

function reportError(action: string, error: unknown) {
  toastStore.pushLocal('error', `${action}: ${error instanceof Error ? error.message : String(error)}`);
}

async function selectVisit(visit: AgentsWorkspaceVisit): Promise<boolean> {
  if (visit.sessionId && !await agentStore.requestSwitchSession(visit.sessionId)) return false;
  if (visit.childId) {
    if (!agentStore.openSubagentSession(visit.childId)) {
      toastStore.pushLocal('warning', 'This child conversation is no longer available in the selected session.');
      return false;
    }
  } else agentStore.closeSubagentSession();
  return true;
}

function AgentsWindowInner() {
  const app = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const agent = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const ui = useSyncExternalStore(agentsWorkspaceStore.subscribe, agentsWorkspaceStore.getSnapshot);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [connectOpen, setConnectOpen] = useState(false);
  const [historyBusy, setHistoryBusy] = useState(false);
  const [requestedTab, setRequestedTab] = useState<{ tab: 'files' | 'changes' } | null>(null);
  const historyLock = useRef(false);
  const [navigationTarget, setNavigationTarget] = useState<AgentsNavigationTarget | null>(null);
  const navigationVersion = useRef(0);
  const settings = app.bootstrap?.settings ?? null;
  const workspaceId = app.bootstrap?.workspace.id ?? null;
  const ready = app.phase === 'ready';
  const commands = useMemo(() => buildCommandList(settings, 'agents'), [settings]);

  useEffect(() => {
    agentsWorkspaceStore.setWorkspace(workspaceId);
  }, [workspaceId]);

  const navigate = useCallback((page: AgentsWorkspacePage, sessionId?: string) => {
    const selected = agentStore.getSnapshot();
    agentsWorkspaceStore.visit({
      page, sessionId: sessionId ?? (selected.activeSessionId || null),
      childId: page === 'conversation' ? selected.activeSubagentId : null,
    });
    if (agentsWorkspaceStore.getSnapshot().compactPanel === 'navigation') agentsWorkspaceStore.closeCompactPanel();
  }, []);

  // Record actual selected parent/child visits, not unrelated bootstrap updates.
  useEffect(() => {
    if (!ready || historyLock.current || ui.workspaceId !== workspaceId || navigationTarget) return;
    if (ui.historyIndex < 0 || ui.page === 'conversation') navigate(ui.page);
  }, [ready, workspaceId, ui.workspaceId, ui.page, ui.historyIndex, agent.activeSessionId, agent.activeSubagentId, navigationTarget, navigate]);

  useEffect(() => {
    if (!ready || !isNativeFrontend()) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    const start = async () => {
      try {
        // Register first: a live navigation event must not be lost during ready.
        const unsubscribe = await subscribeAgentsNavigation((target) => {
          navigationVersion.current += 1;
          if (!disposed) setNavigationTarget(target);
        });
        if (disposed) { unsubscribe(); return; }
        stop = unsubscribe;
        const version = navigationVersion.current;
        const target = await agentsWindowReady();
        if (!disposed && target && version === navigationVersion.current) setNavigationTarget(target);
      } catch (error) {
        if (!disposed) reportError('Could not initialize Agents window navigation', error);
      }
    };
    void start();
    return () => { disposed = true; stop?.(); };
  }, [ready]);

  useEffect(() => {
    if (!ready || !navigationTarget || historyLock.current) return;
    if (navigationTarget.workspace_id !== null && navigationTarget.workspace_id !== workspaceId) return;
    historyLock.current = true;
    setHistoryBusy(true);
    const workspace = workspaceId;
    const version = navigationVersion.current;
    const visit: AgentsWorkspaceVisit = {
      page: 'conversation', sessionId: navigationTarget.session_id || null, childId: navigationTarget.child_id || null,
    };
    void selectVisit(visit).then((selected) => {
      if (selected && agentsWorkspaceStore.getSnapshot().workspaceId === workspace && version === navigationVersion.current) navigate('conversation');
    }).catch((error) => { reportError('Could not select Agents conversation', error); }).finally(() => {
      historyLock.current = false;
      setHistoryBusy(false);
      setNavigationTarget((current) => current === navigationTarget ? null : current);
    });
  }, [ready, workspaceId, navigationTarget, navigate, historyBusy]);

  const goHistory = useCallback(async (delta: -1 | 1) => {
    if (historyLock.current) return;
    const target = agentsWorkspaceStore.historyTarget(delta);
    if (!target) return;
    const workspace = agentsWorkspaceStore.getSnapshot().workspaceId;
    historyLock.current = true;
    setHistoryBusy(true);
    try {
      if (await selectVisit(target) && agentsWorkspaceStore.getSnapshot().workspaceId === workspace) {
        agentsWorkspaceStore.commitHistory(delta);
      }
    } catch (error) {
      reportError('Could not navigate conversation history', error);
    } finally {
      historyLock.current = false;
      setHistoryBusy(false);
    }
  }, []);

  const openFolder = useCallback(async () => {
    const context = currentEnvelopeContext();
    if (!context) return;
    try {
      const path = await pickFolder();
      if (path) await openFolderAt(path, context);
    } catch (error) { reportError('Could not open folder', error); }
  }, []);
  const returnEditor = useCallback(() => {
    void focusEditorWindow().catch((error) => reportError('Could not focus editor window', error));
  }, []);
  const agentsCommand = useCallback((command: AgentsCommand) => {
    switch (command) {
      case 'toggle-navigation': agentsWorkspaceStore.toggleNavigation(); return;
      case 'toggle-context': agentsWorkspaceStore.toggleContext(); return;
      case 'show-files': agentsWorkspaceStore.setContextTab('files'); setRequestedTab({ tab: 'files' }); return;
      case 'show-changes': agentsWorkspaceStore.setContextTab('changes'); setRequestedTab({ tab: 'changes' }); return;
      case 'return-editor': returnEditor(); return;
      case 'new-session':
        void agentStore.requestNewSession().then((created) => { if (created) navigate('conversation'); });
    }
  }, [navigate, returnEditor]);
  const onCommand = useCallback((id: CommandId) => {
    runCommandAction(id, {
      layout: () => null, toggle: () => undefined, saveLayout: () => undefined,
      openPalette: () => setPaletteOpen(true), agentsCommand,
      openFolder: () => void openFolder(),
       closeWindow: () => { if (isNativeFrontend()) void getCurrentWindow().close().catch((error) => reportError('Could not close window', error)); },
      openDocumentation: () => {
        const context = currentEnvelopeContext();
        if (context) void openExternal('https://github.com/codeREDxbt/Skarn/tree/main/docs', context).catch((error) => reportError('Could not open documentation', error));
      },
    });
  }, [agentsCommand, openFolder]);
  const closePalette = useCallback(() => setPaletteOpen(false), []);
  useShortcuts({ settings, enabled: ready && !connectOpen, paletteOpen, onCommand, onClosePalette: closePalette, scope: 'agents' });
  useEffect(() => {
    const connect = () => setConnectOpen(true);
    window.addEventListener('selora:open-connect', connect);
    return () => window.removeEventListener('selora:open-connect', connect);
  }, []);

  if (app.phase === 'error') return <main className="boot-screen" role="alert"><p>{app.error ?? 'Bootstrap failed'}</p><button type="button" onClick={() => void retryBootstrap()}>Reconnect</button></main>;
  if (!ready || !app.bootstrap) return <main className="boot-screen" aria-live="polite">Connecting to Rust core…</main>;
  const bootstrap = app.bootstrap;
  const sessionTitle = agent.sessions.find((session) => session.id === agent.activeSessionId)?.title ?? 'Conversation';
  const title = ui.page === 'conversation' ? sessionTitle : ui.page === 'automations' ? 'Automations' : 'Customizations';
  return <>
    <AgentsWorkspaceShell
      header={<AgentsWorkspaceHeader bootstrap={bootstrap} title={title}
        canGoBack={ui.historyIndex > 0} canGoForward={ui.historyIndex < ui.history.length - 1} historyBusy={historyBusy}
        navigationOpen={ui.viewportWidth < AGENTS_LAYOUT_BOUNDS.navigationBreakpoint ? ui.compactPanel === 'navigation' : ui.navigationOpen}
        contextOpen={ui.viewportWidth < AGENTS_LAYOUT_BOUNDS.contextBreakpoint ? ui.compactPanel === 'context' : ui.contextOpen}
        onBack={() => void goHistory(-1)} onForward={() => void goHistory(1)}
        onToggleNavigation={() => agentsWorkspaceStore.toggleNavigation()} onToggleContext={() => agentsWorkspaceStore.toggleContext()}
        onOpenPalette={() => setPaletteOpen(true)} onOpenFolder={() => void openFolder()} onReturnEditor={returnEditor} />}
      navigation={<AgentsNavigationRail page={ui.page} onNavigate={navigate} onSelectSession={(id) => navigate('conversation', id)} />}
      conversation={<Suspense fallback={<p className="agents-workspace__empty">Loading…</p>}>
        {ui.page === 'conversation' ? <AgentConversationSurface bootstrap={bootstrap} presentation="agents" />
          : ui.page === 'automations' ? <AgentsAutomationsView />
          : <AgentsCustomizationsView bootstrap={bootstrap} onOpenConnect={() => setConnectOpen(true)} />}
      </Suspense>}
      context={<Suspense fallback={null}><AgentsContextPane bootstrap={bootstrap} requestedTab={requestedTab} /></Suspense>}
    />
    <CommandPalette open={paletteOpen} commands={commands} onRun={onCommand} onClose={closePalette} />
    {connectOpen && <Suspense fallback={null}><ConnectModal open onClose={() => setConnectOpen(false)} bootstrap={bootstrap} /></Suspense>}
    <ToastViewport />
  </>;
}

export function AgentsWindowApp() {
  return <AppRuntime><AgentsWindowInner /></AppRuntime>;
}
