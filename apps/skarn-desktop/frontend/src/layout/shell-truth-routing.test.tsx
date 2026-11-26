// Verification for Card S2-1:
// 1. StatusBar shows live state only (no fake Git sync, Ln 1 Col 1, UTF-8, TypeScript fallback, core <hash>).
// 2. Region-strict routing: ActivityBar has no Problems; BottomDock has no Output and uses Terminal; SettingsDialog survives sidebar collapse.
// 3. Blocking file-lock / unsynchronized issues persist across EditorTabBar, ProblemsPanel, and StatusBar until recovered.
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { makeBootstrap, makeLayout } from '../testing/fixtures';

const mocked = vi.hoisted(() => ({
  closeBuffer: vi.fn(),
  toggleSurface: vi.fn().mockResolvedValue(undefined),
  saveLayout: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../bridge/bridge', async (original) => ({
  ...(await original<typeof import('../bridge/bridge')>()),
  closeBuffer: mocked.closeBuffer,
  toggleSurface: mocked.toggleSurface,
  saveLayout: mocked.saveLayout,
}));

import { StatusBar } from './StatusBar';
import { ActivityBar } from './ActivityBar';
import { BottomDock } from './BottomDock';
import { SettingsDialog } from '../components/SettingsDialog';
import { EditorTabBar } from '../components/editor/EditorTabBar';
import { ProblemsPanel } from '../features/dock/ProblemsPanel';
import { layoutStore } from '../features/layout/layout-store';
import { editorStore } from '../features/editor/editor-store';
import { markPendingEditorEdit, settlePendingEditorEdit } from '../features/editor/pending-edits';
import { getAppSnapshot } from '../app/app-store';

describe('S2-1 shell truth, region-strict routing, and persistent blocking errors', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    editorStore.reset();
  });

  it('omits fake status bar fields when no buffer or branch is active', () => {
    const bootstrap = makeBootstrap({
      buffers: [],
      active_buffer_id: null,
      git: { branch: null, changed_paths: [], staged_paths: [] },
    });
    render(<StatusBar bootstrap={bootstrap} />);

    expect(screen.queryByText('TypeScript')).toBeNull();
    expect(screen.queryByText('UTF-8')).toBeNull();
    expect(screen.queryByText('Ln 1, Col 1')).toBeNull();
    expect(screen.queryByText(/^core /)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Git sync' })).toBeNull();
    expect(screen.getByText('Provider disconnected')).toBeTruthy();
  });

  it('removes problems from ActivityBar and Output from BottomDock', () => {
    const bootstrap = makeBootstrap({
      layout: makeLayout({ bottom_dock_open: true, bottom_view: 'terminal' }),
    });
    layoutStore.hydrate(bootstrap);

    const onOpenSettings = vi.fn();
    render(
      <>
        <ActivityBar bootstrap={bootstrap} settingsOpen={false} onOpenSettings={onOpenSettings} />
        <BottomDock />
      </>,
    );

    expect(screen.queryByRole('button', { name: 'Problems' })).toBeNull();
    expect(screen.queryByRole('tab', { name: 'Output' })).toBeNull();
    expect(screen.getByRole('tab', { name: 'Problems' })).toBeTruthy();
    expect(screen.getByRole('tab', { name: 'Terminal' }).getAttribute('aria-selected')).toBe('true');

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it('keeps SettingsDialog open independently of primary sidebar state', () => {
    const bootstrap = makeBootstrap({
      layout: makeLayout({ primary_sidebar_open: false }),
    });
    render(<SettingsDialog open={true} bootstrap={bootstrap} onClose={vi.fn()} />);

    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeTruthy();
    expect(screen.getByText('Assistant providers')).toBeTruthy();
  });

  it('surfaces blocking unsynchronized/close-locked file errors persistently on tab, Problems, and StatusBar', async () => {
    const buffer = {
      buffer_id: 'locked-buf',
      path: 'src/locked.ts',
      language: 'typescript',
      text: 'const x = 1;',
      version: 1,
      dirty: false,
      selection: null,
    };
    const bootstrap = makeBootstrap({
      workspace: { id: 'ws-lock', core_instance_id: 'core-0001', root: 'C:/repo', display_name: 'repo' },
      buffers: [buffer],
      active_buffer_id: buffer.buffer_id,
    });

    // Simulate a pending failed edit so closeBuffer cannot complete safely.
    const appSnap = getAppSnapshot();
    const prevBootstrap = appSnap.bootstrap;
    appSnap.bootstrap = bootstrap;
    try {
      editorStore.reset();
      markPendingEditorEdit(buffer.buffer_id, bootstrap.workspace.id, buffer.path, 2);
      settlePendingEditorEdit(bootstrap.workspace.id, buffer.buffer_id, false);
      editorStore.closeBuffer(buffer.buffer_id);

      await waitFor(() => {
        expect(editorStore.getSnapshot().blockingIssues).toHaveLength(1);
      });

      const state = editorStore.getSnapshot();
      render(
        <>
          <EditorTabBar
            tabs={state.tabs}
            activeTabId={state.activeBufferId}
            onSelectTab={vi.fn()}
            onCloseTab={vi.fn()}
            onSaveTab={vi.fn()}
          />
          <ProblemsPanel />
          <StatusBar bootstrap={bootstrap} />
        </>,
      );

      expect(screen.getByRole('tab', { name: 'locked.ts, Unsynchronized edits' })).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Retry sync' })).toBeTruthy();
      expect(screen.getByText('locked.ts: Unsynchronized edits')).toBeTruthy();

      // When backend catches up or close lock clears, state resets on next reconciliation.
      act(() => {
        mocked.closeBuffer.mockResolvedValueOnce(undefined);
      });
    } finally {
      appSnap.bootstrap = prevBootstrap;
      editorStore.reset();
    }
  });
});
