import './app/run-storage-migration';
// Skarn — Phase Z2 entry point.
//
// Style imports follow the single convention (ARCHITECTURE §8.3, DESIGN
// §13.1): base.css (which @imports tokens.css) first, then component
// stylesheets. The Z1 one-file styles.css is deleted — its hardcoded
// literals are exactly what the token layer replaces.
//
// Region stylesheets belong to their (lazily loaded) region component:
// sidebar.css is imported by PrimarySidebar and dock.css by BottomDock, so a
// closed region never ships that CSS in first paint. agent.css stays eager:
// the sidebar's agent view renders AgentSessionRail, which can be on screen
// from the first frame.
import React, { lazy, Suspense } from 'react';
import { createRoot } from 'react-dom/client';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { isNativeFrontend } from './app/surface';
import './styles/base.css';
import './styles/animations.css';
import './styles/shell.css';
import './styles/theme-galaxy.css';
import './styles/agent.css';
import './styles/editor.css';

const App = lazy(() => import('./app/App').then((module) => ({ default: module.App })));
const AgentsWindowApp = lazy(() => import('./features/agents-workspace/AgentsWindowApp').then((module) => ({ default: module.AgentsWindowApp })));
// Native identity is authoritative. Query routing is only for browser previews.
const agentsSurface = isNativeFrontend()
  ? getCurrentWindow().label === 'agents'
  : new URLSearchParams(window.location.search).get('surface') === 'agents';

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Suspense fallback={<main className="boot-screen" aria-live="polite">Loading Skarn…</main>}>
      {agentsSurface ? <AgentsWindowApp /> : <App />}
    </Suspense>
  </React.StrictMode>,
);
