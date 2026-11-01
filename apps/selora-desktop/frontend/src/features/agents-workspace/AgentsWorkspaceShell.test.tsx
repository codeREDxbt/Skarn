import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { agentsWorkspaceStore } from './agents-workspace-store';
import { AgentsWorkspaceShell } from './AgentsWorkspaceShell';

beforeEach(() => {
  localStorage.clear();
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 1400 });
  agentsWorkspaceStore.setWorkspace('test-shell-workspace');
  agentsWorkspaceStore.setViewportWidth(1400);
});

afterEach(cleanup);

describe('AgentsWorkspaceShell', () => {
  it('renders desktop grid with navigation, conversation, context, and resize handles', () => {
    render(
      <AgentsWorkspaceShell
        header={<header>Header</header>}
        navigation={<nav>Navigation Content</nav>}
        conversation={<main>Conversation Content</main>}
        context={<aside>Context Content</aside>}
      />
    );

    expect(screen.getByText('Header')).toBeTruthy();
    expect(screen.getByText('Conversation Content')).toBeTruthy();
    expect(screen.getByText('Navigation Content')).toBeTruthy();
    expect(screen.getByText('Context Content')).toBeTruthy();

    const handles = screen.getAllByRole('separator');
    expect(handles).toHaveLength(2);
    expect(handles[0].getAttribute('aria-label')).toBe('Resize navigation');
    expect(handles[1].getAttribute('aria-label')).toBe('Resize context');
  });

  it('supports keyboard navigation on resize handles', () => {
    render(
      <AgentsWorkspaceShell
        header={<header>Header</header>}
        navigation={<nav>Navigation Content</nav>}
        conversation={<main>Conversation Content</main>}
        context={<aside>Context Content</aside>}
      />
    );

    const handle = screen.getByRole('separator', { name: 'Resize navigation' });
    const initialWidth = agentsWorkspaceStore.getSnapshot().navigationWidth;

    // ArrowRight expands navigation
    fireEvent.keyDown(handle, { key: 'ArrowRight' });
    expect(agentsWorkspaceStore.getSnapshot().navigationWidth).toBeGreaterThan(initialWidth);

    // ArrowLeft shrinks navigation
    fireEvent.keyDown(handle, { key: 'ArrowLeft' });
    expect(agentsWorkspaceStore.getSnapshot().navigationWidth).toBe(initialWidth);

    // Home snaps to minimum
    fireEvent.keyDown(handle, { key: 'Home' });
    expect(agentsWorkspaceStore.getSnapshot().navigationWidth).toBe(200);

    // End snaps to maximum
    fireEvent.keyDown(handle, { key: 'End' });
    expect(agentsWorkspaceStore.getSnapshot().navigationWidth).toBe(360);

    // Double-click resets to default (248px)
    fireEvent.doubleClick(handle);
    expect(agentsWorkspaceStore.getSnapshot().navigationWidth).toBe(248);
  });

  it('renders modal drawer and scrim in compact viewport mode and closes on scrim click', () => {
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 600 });
    agentsWorkspaceStore.setViewportWidth(600);
    agentsWorkspaceStore.toggleNavigation();

    render(
      <AgentsWorkspaceShell
        header={<header>Header</header>}
        navigation={<nav>Drawer Navigation</nav>}
        conversation={<main>Conversation Content</main>}
        context={<aside>Drawer Context</aside>}
      />
    );

    expect(screen.getByRole('dialog')).toBeTruthy();
    expect(screen.getByRole('dialog').getAttribute('aria-modal')).toBe('true');
    expect(screen.getByText('Drawer Navigation')).toBeTruthy();

    const scrim = screen.getByRole('button', { name: 'Close side panel' });
    fireEvent.click(scrim);
    expect(agentsWorkspaceStore.getSnapshot().compactPanel).toBeNull();
  });

  it('closes compact drawer when pressing Escape', () => {
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, value: 600 });
    agentsWorkspaceStore.setViewportWidth(600);
    agentsWorkspaceStore.toggleNavigation();

    render(
      <AgentsWorkspaceShell
        header={<header>Header</header>}
        navigation={<nav>Drawer Navigation</nav>}
        conversation={<main>Conversation Content</main>}
        context={<aside>Drawer Context</aside>}
      />
    );

    expect(screen.getByRole('dialog')).toBeTruthy();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(agentsWorkspaceStore.getSnapshot().compactPanel).toBeNull();
  });
});
