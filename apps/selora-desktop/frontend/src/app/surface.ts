import { getCurrentWindow } from '@tauri-apps/api/window';

export type FrontendSurface = { kind: 'main' | 'agents'; id: string };

export function isNativeFrontend(): boolean {
  if (typeof window === 'undefined') return false;
  const internals = (window as Window & {
    __TAURI_INTERNALS__?: { invoke?: unknown };
  }).__TAURI_INTERNALS__;
  return typeof internals?.invoke === 'function';
}

/** Native window identity is authoritative; URLs select surfaces only in previews. */
export function getFrontendSurface(): FrontendSurface {
  if (isNativeFrontend()) {
    const id = getCurrentWindow().label;
    return { kind: id === 'agents' ? 'agents' : 'main', id };
  }
  if (typeof window !== 'undefined') {
    const url = new URL(window.location.href);
    const agents = url.pathname.replace(/\/$/, '').endsWith('/agents') ||
      url.searchParams.get('window') === 'agents' || url.searchParams.get('surface') === 'agents' ||
      url.hash === '#/agents';
    if (agents) return { kind: 'agents', id: 'agents' };
  }
  return { kind: 'main', id: 'main' };
}
