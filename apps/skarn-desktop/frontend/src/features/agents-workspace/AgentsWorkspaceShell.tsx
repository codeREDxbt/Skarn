import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type CSSProperties, type ReactNode } from 'react';
import { AGENTS_LAYOUT_BOUNDS, agentsWorkspaceStore, clampAgentsWidth, type AgentsWorkspacePanel } from './agents-workspace-store';
import './agents-workspace.css';

export type AgentsWorkspaceShellProps = {
  header: ReactNode;
  navigation: ReactNode;
  conversation: ReactNode;
  context: ReactNode;
};

function WorkspaceResizeHandle({ panel, displayedWidth, maxWidth }: {
  panel: AgentsWorkspacePanel; displayedWidth: number; maxWidth: number;
}) {
  const drag = useRef<{ id: number; x: number; size: number } | null>(null);
  const width = displayedWidth;
  const bounds = AGENTS_LAYOUT_BOUNDS[panel];
  const max = Math.max(bounds.min, Math.min(bounds.max, maxWidth));
  const clamp = (value: number) => Math.min(max, clampAgentsWidth(panel, value));
  const finish = (event: React.PointerEvent<HTMLDivElement>, cancel = false) => {
    const start = drag.current;
    if (!start || start.id !== event.pointerId) return;
    const grid = event.currentTarget.parentElement;
    const preview = grid?.style.getPropertyValue(`--agents-${panel}-width`);
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    grid?.removeAttribute('data-resizing');
    agentsWorkspaceStore.resize(panel, cancel ? start.size : clamp(Number.parseFloat(preview ?? '') || start.size));
    // Keep the inline variable synchronized even when committing the same size.
    grid?.style.setProperty(`--agents-${panel}-width`, `${cancel ? start.size : clamp(Number.parseFloat(preview ?? '') || start.size)}px`);
  };
  return <div
    className={`agents-workspace__resize agents-workspace__resize--${panel}`}
    role="separator" tabIndex={0} aria-orientation="vertical"
    aria-label={`Resize ${panel}`} aria-valuemin={bounds.min} aria-valuemax={Math.round(max)} aria-valuenow={Math.round(Math.min(width, max))}
    onPointerDown={(event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      drag.current = { id: event.pointerId, x: event.clientX, size: width };
      event.currentTarget.setPointerCapture(event.pointerId);
      event.currentTarget.parentElement?.setAttribute('data-resizing', panel);
    }}
    onPointerMove={(event) => {
      const start = drag.current;
      if (!start || start.id !== event.pointerId) return;
      const delta = (event.clientX - start.x) * (panel === 'navigation' ? 1 : -1);
      event.currentTarget.parentElement?.style.setProperty(`--agents-${panel}-width`, `${clamp(start.size + delta)}px`);
    }}
    onPointerUp={(event) => finish(event)} onPointerCancel={(event) => finish(event, true)}
    onLostPointerCapture={(event) => finish(event, true)}
    onDoubleClick={() => agentsWorkspaceStore.resize(panel, clamp(bounds.default))}
    onKeyDown={(event) => {
      let size: number;
      if (event.key === 'Home') size = bounds.min;
      else if (event.key === 'End') size = max;
      else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        size = width + (event.key === 'ArrowRight' ? 1 : -1) * (panel === 'navigation' ? 1 : -1) * (event.shiftKey ? 16 : 4);
      } else return;
      event.preventDefault();
      agentsWorkspaceStore.resize(panel, clamp(size));
    }}
  />;
}

export function AgentsWorkspaceShell({ header, navigation, conversation, context }: AgentsWorkspaceShellProps) {
  const state = useSyncExternalStore(agentsWorkspaceStore.subscribe, agentsWorkspaceStore.getSnapshot);
  const root = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const drawer = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!state.compactPanel) return;
    const previousFocus = document.activeElement;
    drawer.current?.querySelector<HTMLElement>('button, input, [tabindex="0"]')?.focus();
    const onEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      agentsWorkspaceStore.closeCompactPanel();
    };
    window.addEventListener('keydown', onEscape);
    return () => {
      window.removeEventListener('keydown', onEscape);
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, [state.compactPanel]);
  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const measure = (value: number) => { setWidth(value); agentsWorkspaceStore.setViewportWidth(value); };
    measure(element.clientWidth);
    const observer = new ResizeObserver(([entry]) => measure(entry.contentRect.width));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const navigationVisible = state.navigationOpen && width >= AGENTS_LAYOUT_BOUNDS.navigationBreakpoint;
  const contextVisible = state.contextOpen && width >= AGENTS_LAYOUT_BOUNDS.contextBreakpoint;
  // Responsive geometry never overwrites the user's saved desktop sizes.
  const room = Math.max(0, width - AGENTS_LAYOUT_BOUNDS.conversationMin - AGENTS_LAYOUT_BOUNDS.handle * (Number(navigationVisible) + Number(contextVisible)));
  const contextWidth = Math.min(state.contextWidth, Math.max(AGENTS_LAYOUT_BOUNDS.context.min, room - (navigationVisible ? state.navigationWidth : 0)));
  const navigationWidth = Math.min(state.navigationWidth, Math.max(AGENTS_LAYOUT_BOUNDS.navigation.min, room - (contextVisible ? contextWidth : 0)));
  const style = {
    '--agents-navigation-width': `${navigationWidth}px`, '--agents-context-width': `${contextWidth}px`,
    '--agents-handle-width': `${AGENTS_LAYOUT_BOUNDS.handle}px`,
    gridTemplateColumns: `${navigationVisible ? 'var(--agents-navigation-width) var(--agents-handle-width) ' : ''}minmax(0, 1fr)${contextVisible ? ' var(--agents-handle-width) var(--agents-context-width)' : ''}`,
  } as CSSProperties;
  return <div className="agents-workspace" ref={root}>
    {header}
    <div className="agents-workspace__grid" style={style}>
      {navigationVisible && <><aside id="agents-workspace-navigation" className="agents-workspace__navigation" aria-label="Agents navigation">{navigation}</aside><WorkspaceResizeHandle panel="navigation" displayedWidth={navigationWidth} maxWidth={room - (contextVisible ? contextWidth : 0)} /></>}
      <main className="agents-workspace__conversation" id="agents-workspace-content">{conversation}</main>
      {contextVisible && <><WorkspaceResizeHandle panel="context" displayedWidth={contextWidth} maxWidth={room - (navigationVisible ? navigationWidth : 0)} /><aside id="agents-workspace-context" className="agents-workspace__context" aria-label="Workspace context">{context}</aside></>}
    </div>
    {state.compactPanel && <>
      <button type="button" className="agents-workspace__scrim" aria-label="Close side panel" onClick={() => agentsWorkspaceStore.closeCompactPanel()} />
       <aside ref={drawer} role="dialog" aria-modal="true" className={`agents-workspace__drawer agents-workspace__drawer--${state.compactPanel}`} aria-labelledby="agents-workspace-drawer-title"
        onKeyDown={(event) => {
          if (event.key !== 'Tab') return;
          const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"], a[href]'));
          const first = controls[0];
          const last = controls.at(-1);
          if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
          else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
        }}>
         <div className="agents-workspace__drawer-header">
           <h2 id="agents-workspace-drawer-title">{state.compactPanel === 'navigation' ? 'Agents navigation' : 'Workspace context'}</h2>
           <button type="button" className="agents-workspace__drawer-close" onClick={() => agentsWorkspaceStore.closeCompactPanel()}>Close panel</button>
         </div>
        {state.compactPanel === 'navigation' ? navigation : context}
      </aside>
    </>}
  </div>;
}
