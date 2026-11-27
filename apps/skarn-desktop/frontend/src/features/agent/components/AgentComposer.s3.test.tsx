import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { makeBootstrap } from '../../../testing/fixtures';
import { agentStore } from '../agent-store';
import { clearAgentDraft } from '../draft-store';
import { AgentComposer } from './AgentComposer';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const WORKSPACE_ID = 'ws-composer-s3';
const SESSION_ID = 'sess-composer-s3';

function setupWorkspace(trusted = false) {
  agentStore.reset();
  clearAgentDraft(WORKSPACE_ID, SESSION_ID, 'main');
  agentStore.hydrate(
    makeBootstrap({
      workspace: {
        id: WORKSPACE_ID,
        core_instance_id: 'core-0001',
        root: 'C:/repo',
        display_name: 'repo',
      },
      models: [{ id: 'gpt-4o', display_name: 'GPT-4o', available: true, tool_capability: 'supported' }],
      active_model_id: 'gpt-4o',
      settings: {
        reduced_motion: false,
        theme: 'dark',
        keybind_command_palette: 'Ctrl+Shift+P',
        keybind_toggle_chat: 'Ctrl+L',
        keybind_fuzzy_find: 'Ctrl+P',
        keybind_save: 'Ctrl+S',
        editor_font_size: 14,
        editor_tab_size: 4,
        editor_word_wrap: false,
        workspace_trusted: trusted,
      },
      rules_in_effect: [
        {
          path: 'C:/Users/me/AppData/Roaming/skarn/AGENTS.md',
          scope: 'global',
          active: true,
          digest: 'abc123def456',
        },
        {
          path: 'C:/repo/AGENTS.md',
          scope: 'workspace',
          active: trusted,
          inactive_reason: trusted ? null : 'workspace_untrusted',
        },
        {
          path: 'C:/repo/.skarn/rules',
          scope: 'workspace',
          active: trusted,
          inactive_reason: trusted ? null : 'workspace_untrusted',
        },
      ],
      chat: {
        sessions: [{ session_id: SESSION_ID, title: 'S3 Session', active: true, message_ids: [] }],
        messages: [],
        active_session_id: SESSION_ID,
        active_request_id: null,
        active_turn_id: null,
        pending_approval: null,
      },
    }),
  );
}

describe('AgentComposer Stage 3 invariants', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue({ acknowledged: true });
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('offers all 4 canonical permission modes and cycles them with Shift+Tab', () => {
    setupWorkspace(true);
    const { rerender } = render(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode={agentStore.getSnapshot().permissionMode}
        reasoningEffort="standard"
        promptHistory={[]}
      />,
    );

    fireEvent.click(screen.getByTitle('Toggle permission policy (Shift+Tab to cycle)'));
    const menu = screen.getByRole('menu');
    expect(within(menu).getByText('Read-only')).toBeTruthy();
    expect(within(menu).getByText('Ask before changes')).toBeTruthy();
    expect(within(menu).getByText('Auto-edit')).toBeTruthy();
    expect(within(menu).getByText('Full access')).toBeTruthy();

    const textarea = screen.getByRole('textbox', { name: 'Composer prompt input' });
    // Default is ask_before_changes -> auto_edit -> full_access -> read_only -> ask_before_changes
    fireEvent.keyDown(textarea, { key: 'Tab', shiftKey: true });
    expect(agentStore.getSnapshot().permissionMode).toBe('auto_edit');

    rerender(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode={agentStore.getSnapshot().permissionMode}
        reasoningEffort="standard"
        promptHistory={[]}
      />,
    );
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Composer prompt input' }), { key: 'Tab', shiftKey: true });
    expect(agentStore.getSnapshot().permissionMode).toBe('full_access');

    rerender(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode={agentStore.getSnapshot().permissionMode}
        reasoningEffort="standard"
        promptHistory={[]}
      />,
    );
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Composer prompt input' }), { key: 'Tab', shiftKey: true });
    expect(agentStore.getSnapshot().permissionMode).toBe('read_only');

    rerender(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode={agentStore.getSnapshot().permissionMode}
        reasoningEffort="standard"
        promptHistory={[]}
      />,
    );
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Composer prompt input' }), { key: 'Tab', shiftKey: true });
    expect(agentStore.getSnapshot().permissionMode).toBe('ask_before_changes');
  });

  it('sends typed AttachmentDto[] without injecting ATTACHMENT_HEADER prose into prompt or transcript', () => {
    setupWorkspace(true);
    const { container } = render(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode="ask_before_changes"
        reasoningEffort="standard"
        promptHistory={[]}
      />,
    );

    const composer = container.querySelector('.agent-composer')!;
    fireEvent.drop(composer, {
      dataTransfer: {
        files: [{ path: 'C:/repo/src/main.ts' }, { path: 'C:/repo/src/lib.ts' }],
      },
    });

    const input = screen.getByRole('textbox', { name: 'Composer prompt input' });
    fireEvent.change(input, { target: { value: 'Inspect these two files' } });
    fireEvent.click(screen.getByRole('button', { name: 'Send' }));

    expect(invoke).toHaveBeenCalledWith('dispatch_command', {
      envelope: expect.objectContaining({
        payload: {
          type: 'send_agent',
          payload: {
            request_id: expect.any(String),
            request: expect.objectContaining({
              prompt: 'Inspect these two files',
              mode: 'ask_before_changes',
              attachments: [
                { path: 'src/main.ts', kind: 'file', label: 'main.ts' },
                { path: 'src/lib.ts', kind: 'file', label: 'lib.ts' },
              ],
            }),
          },
        },
      }),
    });

    const sentUserMsg = agentStore.getSnapshot().messages.find((m) => m.role === 'user');
    expect(sentUserMsg?.content).toBe('Inspect these two files');
    expect(sentUserMsg?.content).not.toContain('Attached workspace files');
  });

  it('shows active global rules vs untrusted-skipped workspace rules in Rules in effect popover', () => {
    setupWorkspace(false);
    render(
      <AgentComposer
        inFlight={false}
        streamingActivityLevel={0}
        permissionMode="ask_before_changes"
        reasoningEffort="standard"
        promptHistory={[]}
      />,
    );

    const rulesBtn = screen.getByRole('button', { name: /Rules in effect \(1\)/i });
    fireEvent.click(rulesBtn);

    const dialog = screen.getByRole('dialog', { name: 'Rules in effect' });
    expect(within(dialog).getByText('Untrusted workspace')).toBeTruthy();
    expect(within(dialog).getByText('Active')).toBeTruthy();
    expect(within(dialog).getAllByText('Skipped (untrusted)')).toHaveLength(2);

    // Live trust update activates workspace rules
    act(() => {
      agentStore.handleRulesLoaded({
        workspace_trusted: true,
        sources: [
          { path: 'C:/Users/me/AppData/Roaming/skarn/AGENTS.md', scope: 'global', active: true },
          { path: 'C:/repo/AGENTS.md', scope: 'workspace', active: true },
          { path: 'C:/repo/.skarn/rules', scope: 'workspace', active: true },
        ],
      });
    });

    expect(screen.getByRole('button', { name: /Rules in effect \(3\)/i })).toBeTruthy();
    expect(within(dialog).getByText('Trusted workspace')).toBeTruthy();
    expect(within(dialog).getAllByText('Active')).toHaveLength(3);
    expect(within(dialog).queryByText('Skipped (untrusted)')).toBeNull();
  });
});
