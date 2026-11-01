// Selora IDE — Phase Z2 app providers.
//
// `AppRuntime` owns the app-lifetime side effects: it starts the
// bootstrap/replay flow once (StrictMode-safe) and routes the bridge's
// command-error stream into the toast surface (ARCHITECTURE §4.4).
import { useEffect, type ReactNode } from 'react';
import { getAppSnapshot, startApp } from './app-store';
import { onBridgeCommandError, subscribeMainSessionNavigation } from '../bridge/bridge';
import { toastStore } from '../features/toasts/toast-store';
import { getFrontendSurface, isNativeFrontend } from './surface';
import { agentStore } from '../features/agent/agent-store';

export function AppRuntime({ children }: { children: ReactNode }) {
  useEffect(() => {
    void startApp();
  }, []);
  useEffect(
    () =>
      onBridgeCommandError((error) => {
        if (getFrontendSurface().kind === 'main') toastStore.pushLocal('error', error.message);
      }),
    [],
  );
  useEffect(() => {
    if (!isNativeFrontend() || getFrontendSurface().kind !== 'main') return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void subscribeMainSessionNavigation((target) => {
      if (disposed || target.workspace_id !== getAppSnapshot().bootstrap?.workspace.id) return;
      void (async () => {
        const sessionId = target.session_id;
        if (sessionId && !await agentStore.requestSwitchSession(sessionId)) return;
        if (target.child_id) agentStore.openSubagentSession(target.child_id);
      })().catch((error: unknown) => {
        toastStore.pushLocal('error', error instanceof Error ? error.message : String(error));
      });
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    }).catch((error: unknown) => {
      if (!disposed) toastStore.pushLocal('error', error instanceof Error ? error.message : String(error));
    });
    return () => { disposed = true; stop?.(); };
  }, []);
  return <>{children}</>;
}
