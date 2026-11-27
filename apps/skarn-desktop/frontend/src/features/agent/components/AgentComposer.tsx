// Selora IDE — ZCode Parity Hero Composer Card (REVAMP_SPEC §2.1, PHASE_Z §8.1, ZCode v4 parity)
//
// Sacred behavior:
// - Enter submits (when trimmed is non-empty).
// - Shift+Enter inserts newline.
// - Up/Down recalls prompt history on a single-line or empty buffer.
// - Smooth auto-expanding multiline input up to 8 rows.
// - BorderBeam rides perimeter during in-flight runs with dual-beam aurora glow.
// - Toolbar with Iconsax icons: permission mode dropdown, model switcher,
//   reasoning effort, Send/Stop.
//
// Honesty rules:
// - The model list is exactly the backend catalog (bootstrap.models /
//   models_changed). No fallback catalog, no invented context windows or
//   latencies. With no catalog we show a "Choose a provider to start" empty
//   state and disable send.
// - No workspace from Rust means the agent cannot read/write; the composer
//   explains that and disables send instead of inventing a folder.
// - Permission mode labels describe real session behavior: full = auto-apply
//   writes (bash still asks), ask = every write/bash asks, plan = read-only.
import React, { useState, useRef, useEffect, useCallback, useMemo, useSyncExternalStore } from 'react';
import {
  Plus,
  ShieldCheck,
  Lightbulb,
  ShieldAlert,
  ChevronDown,
  Sparkles,
  Zap,
  ArrowUp,
  Square,
  X,
  Check,
  Key,
  DirectSend,
  Search,
  Paperclip,
  FileText,
} from '../../../components/icons';
import { VoiceBeam } from '../presence/VoiceBeam';
import type { AttachmentDto, FileTreeNode, Model, ToolCapability } from '../../../bridge/bridge';
import type { PermissionMode, ReasoningEffort } from '../types';
import { agentStore, getComposerGate, resolveModelId } from '../agent-store';
import { activeEditorContext, editorStore } from '../../editor/editor-store';
import { getAppSnapshot, subscribeApp } from '../../../app/app-store';
import { providerIdForModel } from '../../auth/provider-visibility';
import { safeWorkspaceRelativePath } from './chat-safety';
import { readAgentDraft, writeAgentDraft, clearAgentDraft, type AgentDraft } from '../draft-store';
import './agent-attachments.css';

export type AgentComposerProps = {
  surface?: 'main' | 'agents';
  onPreviewAttachment?: (path: string) => void;
  inFlight: boolean;
  streamingActivityLevel: number;
  permissionMode: PermissionMode;
  reasoningEffort: ReasoningEffort;
  promptHistory: string[];
};

type MenuType = 'mode' | 'model' | 'effort' | null;

type ModelItem = {
  id: string;
  name: string;
  provider: string;
  colorClass: string;
  available: boolean;
  toolCapability: ToolCapability;
  reasoningCapability: 'native' | 'unsupported' | 'unknown';
  reasoningModes: string[];
};

const MODE_OPTIONS: Array<{
  id: PermissionMode;
  label: string;
  description: string;
  icon: typeof ShieldCheck;
  colorClass: string;
}> = [
  {
    id: 'read_only',
    label: 'Read-only',
    description: 'Explores and plans without writing any files.',
    icon: Lightbulb,
    colorClass: 'mode-icon-plan',
  },
  {
    id: 'ask_before_changes',
    label: 'Ask before changes',
    description: 'Every file write and bash command pauses for your approval.',
    icon: ShieldAlert,
    colorClass: 'mode-icon-ask',
  },
  {
    id: 'auto_edit',
    label: 'Auto-edit',
    description:
      'File writes, edits and deletes apply immediately for this session. Bash commands still ask for approval.',
    icon: ShieldCheck,
    colorClass: 'mode-icon-full',
  },
  {
    id: 'full_access',
    label: 'Full access',
    description:
      'File edits and trusted workspace commands run without pausing (high-risk actions still ask).',
    icon: Zap,
    colorClass: 'mode-icon-full',
  },
];

const EFFORT_OPTIONS: Array<{ id: ReasoningEffort; label: string; desc: string }> = [
  { id: 'none', label: 'Off', desc: 'Disable provider thinking when supported' },
  { id: 'max', label: 'Max', desc: 'Thorough, multi-step chain of thought' },
  { id: 'standard', label: 'Standard', desc: 'Balanced speed and analysis' },
  { id: 'quick', label: 'Quick', desc: 'Fast, minimal thought overhead' },
];

/**
 * Attachments are typed workspace-relative references (`AttachmentDto[]`) sent
 * directly in `AgentRequestDto.attachments` without mutating user prompt prose.
 */
const MAX_ATTACHMENTS = 8;

function flattenFileTree(node: FileTreeNode | null, out: string[] = []): string[] {
  if (!node || out.length >= 5000) return out;
  if (node.kind === 'file') out.push(node.path);
  for (const child of node.children ?? []) flattenFileTree(child, out);
  return out;
}

function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

function toAttachmentDtos(paths: readonly string[]): AttachmentDto[] {
  return paths.map((path) => ({
    path,
    kind: 'file',
    label: baseName(path),
  }));
}

/** Provider label derived from the real backend model id (no fake metadata). */
function describeModel(model: Model): ModelItem {
  const idLower = `${model.id} ${model.display_name}`.toLowerCase();
  const available = model.available;
  const toolCapability: ToolCapability = model.tool_capability ?? 'unknown';
  const reasoningCapability = model.reasoning_capability ?? 'unknown';
  const reasoningModes = model.reasoning_modes ?? ['none', 'quick', 'standard', 'max'];
  const providerMeta = (() => {
    if (idLower.includes('claude') || idLower.includes('anthropic')) {
      return { provider: 'Anthropic', colorClass: 'provider-anthropic' };
    }
    if (
      idLower.includes('gpt') ||
      idLower.includes('openai') ||
      idLower.includes('o1') ||
      idLower.includes('o3')
    ) {
      return { provider: 'OpenAI', colorClass: 'provider-openai' };
    }
    if (idLower.includes('deepseek')) {
      return { provider: 'DeepSeek', colorClass: 'provider-deepseek' };
    }
    if (idLower.includes('glm') || idLower.includes('zhipu')) {
      return { provider: 'Zhipu', colorClass: 'provider-zhipu' };
    }
    if (idLower.includes('gemini') || idLower.includes('google')) {
      return { provider: 'Google', colorClass: 'provider-custom' };
    }
    if (idLower.includes('ollama') || idLower.includes('llama') || idLower.includes('qwen')) {
      return { provider: 'Local', colorClass: 'provider-custom' };
    }
    return { provider: 'Provider', colorClass: 'provider-custom' };
  })();
  return {
    id: model.id,
    name: model.display_name || model.id,
    provider: providerMeta.provider,
    colorClass: providerMeta.colorClass,
    available,
    toolCapability,
    reasoningCapability,
    reasoningModes,
  };
}

export const AgentComposer: React.FC<AgentComposerProps> = (props) => {
  const state = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  return <SessionAgentComposer key={`${state.workspace?.id ?? ''}:${state.activeSessionId ?? ''}:${props.surface ?? 'main'}`} {...props} />;
};

const SessionAgentComposer: React.FC<AgentComposerProps> = ({
  surface = 'main',
  onPreviewAttachment,
  inFlight,
  streamingActivityLevel,
  permissionMode,
  reasoningEffort,
  promptHistory,
}) => {
  const storeState = useSyncExternalStore(agentStore.subscribe, agentStore.getSnapshot);
  const appSnapshot = useSyncExternalStore(subscribeApp, getAppSnapshot);
  const availableModels = storeState.availableModels;
  const selectedModelId = storeState.selectedModel;
  const gate = getComposerGate(storeState);

  const workspaceId = storeState.workspace?.id ?? '';
  const sessionId = storeState.activeSessionId ?? '';
  const [draft, setDraft] = useState<AgentDraft>(() => {
    const saved = readAgentDraft(workspaceId, sessionId, surface);
    return { ...saved, attachments: Array.from(new Set(saved.attachments
      .map((path) => safeWorkspaceRelativePath(storeState.workspace?.root ?? null, path))
      .filter((path): path is string => path !== null))).slice(0, MAX_ATTACHMENTS) };
  });
  const draftRef = useRef(draft);
  const updateDraft = useCallback((update: Partial<AgentDraft>) => {
    const next = { ...draftRef.current, ...update, revision: draftRef.current.revision + 1 };
    writeAgentDraft(workspaceId, sessionId, surface, next);
    draftRef.current = readAgentDraft(workspaceId, sessionId, surface);
    setDraft(draftRef.current);
  }, [workspaceId, sessionId, surface]);
  const text = draft.text;
  const attachments = draft.attachments;
  const setText = useCallback((value: string) => updateDraft({ text: value }), [updateDraft]);
  const setAttachments = useCallback((value: string[] | ((current: string[]) => string[])) => {
    updateDraft({ attachments: typeof value === 'function' ? value(draftRef.current.attachments) : value });
  }, [updateDraft]);
  const clearAcceptedDraft = useCallback((revision: number) => {
    if (draftRef.current.revision !== revision) return;
    if (!clearAgentDraft(workspaceId, sessionId, surface, revision)) return;
    const empty = readAgentDraft(workspaceId, sessionId, surface);
    draftRef.current = empty;
    setDraft(empty);
  }, [workspaceId, sessionId, surface]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [savedDraft, setSavedDraft] = useState('');
  const [isFocused, setIsFocused] = useState(false);
  const [activeMenu, setActiveMenu] = useState<MenuType>(null);
  const [modelSearch, setModelSearch] = useState('');
  const [modelError, setModelError] = useState<string | null>(null);
  const [attachOpen, setAttachOpen] = useState(false);
  const [attachQuery, setAttachQuery] = useState('');
  const [rulesOpen, setRulesOpen] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const workspaceRoot = storeState.workspace?.root ?? null;

  const editor = useSyncExternalStore(editorStore.subscribe, editorStore.getSnapshot);

  const workspaceFiles = useMemo(() => {
    const seen = new Set<string>();
    const options: Array<{ path: string; name: string; open: boolean }> = [];
    for (const tab of editor.tabs ?? []) {
      const path = safeWorkspaceRelativePath(workspaceRoot, tab.path);
      if (!path || seen.has(path)) continue;
      seen.add(path);
      options.push({ path, name: tab.filename, open: true });
    }
    for (const rawPath of flattenFileTree(editor.fileTree ?? null)) {
      const path = safeWorkspaceRelativePath(workspaceRoot, rawPath);
      if (!path) continue;
      if (seen.has(path)) continue;
      seen.add(path);
      options.push({ path, name: baseName(path), open: false });
    }
    return options;
  }, [editor.tabs, editor.fileTree, workspaceRoot]);

  useEffect(() => {
    const attach = (event: Event) => {
      const detail = (event as CustomEvent<{ path: string; sessionId: string; surface: string; workspaceId: string }>).detail;
      if (!detail || typeof detail.path !== 'string' || detail.surface !== surface || detail.sessionId !== sessionId || detail.workspaceId !== workspaceId) return;
      const path = safeWorkspaceRelativePath(workspaceRoot, detail.path);
      if (path) setAttachments((current) => Array.from(new Set([...current, path])).slice(0, MAX_ATTACHMENTS));
    };
    window.addEventListener('skarn:attach-agent-file', attach);
    return () => window.removeEventListener('skarn:attach-agent-file', attach);
  }, [workspaceRoot, workspaceId, sessionId, surface, setAttachments]);

  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setIsDragOver(false);
    const dropped = Array.from(event.dataTransfer.files)
      .map((file) => {
        const path = (file as File & { path?: string }).path ?? '';
        if (!/^(?:[A-Za-z]:[\\/]|[\\/])/.test(path)) return null;
        return safeWorkspaceRelativePath(workspaceRoot, path);
      })
      .filter((path): path is string => path !== null);
    setAttachments((current) => Array.from(new Set([...current, ...dropped])).slice(0, MAX_ATTACHMENTS));
  };

  const filteredFiles = useMemo(() => {
    const query = attachQuery.trim().toLowerCase();
    const matches = query
      ? workspaceFiles.filter((file) => file.path.toLowerCase().includes(query))
      : workspaceFiles;
    return matches.slice(0, 80);
  }, [workspaceFiles, attachQuery]);

  const toggleAttachment = useCallback((path: string) => {
    setAttachments((current) => {
      if (current.includes(path)) return current.filter((entry) => entry !== path);
      if (current.length >= MAX_ATTACHMENTS) return current;
      return [...current, path];
    });
  }, [setAttachments]);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const modelSearchInputRef = useRef<HTMLInputElement>(null);
  const modelTriggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const saved = draftRef.current;
    if (saved.selectionStart !== undefined) textareaRef.current?.setSelectionRange(
      saved.selectionStart, saved.selectionEnd ?? saved.selectionStart,
    );
  }, []);

  // Auto-resize textarea with smooth height transition
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    const computedHeight = Math.min(el.scrollHeight, 180);
    el.style.height = `${Math.max(computedHeight, 26)}px`;
  }, [text]);

  // Focus search input when model menu opens
  useEffect(() => {
    if (activeMenu !== 'model') {
      setModelSearch('');
      setModelError(null);
      return;
    }
    const timer = window.setTimeout(() => {
      modelSearchInputRef.current?.focus();
    }, 50);
    return () => window.clearTimeout(timer);
  }, [activeMenu]);

  // Click outside to close any open floating menus
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setActiveMenu(null);
        setAttachOpen(false);
        setRulesOpen(false);
      }
    };
    document.addEventListener('pointerdown', handleClickOutside);
    return () => document.removeEventListener('pointerdown', handleClickOutside);
  }, []);

  // Listen for draft restore events (e.g. from Rewind or Queue edit)
  useEffect(() => {
    const handleSetDraft = (e: Event) => {
      const customEvent = e as CustomEvent<{ text: string; surface?: 'main' | 'agents'; sessionId?: string }>;
      if (customEvent.detail?.surface && customEvent.detail.surface !== surface) return;
      if (customEvent.detail?.sessionId && customEvent.detail.sessionId !== sessionId) return;
      if (!customEvent.detail?.surface && (!document.hasFocus() ||
          !containerRef.current?.closest('[data-presentation]')?.contains(document.activeElement) &&
          !containerRef.current?.contains(document.activeElement))) return;
      if (customEvent.detail && typeof customEvent.detail.text === 'string') {
        setText(customEvent.detail.text);
        setTimeout(() => {
          if (textareaRef.current) {
            textareaRef.current.focus();
            textareaRef.current.selectionStart = textareaRef.current.value.length;
            textareaRef.current.selectionEnd = textareaRef.current.value.length;
          }
        }, 50);
      }
    };
    window.addEventListener('skarn:set-composer-draft', handleSetDraft);
    return () => {
      window.removeEventListener('skarn:set-composer-draft', handleSetDraft);
    };
  }, [surface, sessionId, setText]);

  const handleRefreshModels = useCallback(() => {
    setModelError(null);
    void agentStore.refreshModels().catch((err: unknown) => {
      const message = err instanceof Error ? err.message : String(err ?? '');
      setModelError(message || 'Could not refresh models. Check the provider connection.');
    });
  }, []);

  const handleQueue = useCallback(async () => {
    if (!gate.ready) return;
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    const revision = draftRef.current.revision;
    const context = surface === 'main' ? activeEditorContext() ?? undefined : undefined;
    const attachmentDtos = attachments.length > 0 ? toAttachmentDtos(attachments) : undefined;
    const queued = await agentStore.queuePrompt(trimmed, permissionMode, context, attachmentDtos);
    if (!queued) return;
    clearAcceptedDraft(revision);
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
    setAttachOpen(false);
    setRulesOpen(false);
  }, [gate.ready, text, permissionMode, attachments, surface, clearAcceptedDraft]);

  const handleSteer = useCallback(async () => {
    if (!gate.ready) return;
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    const revision = draftRef.current.revision;
    const attachmentDtos = attachments.length > 0 ? toAttachmentDtos(attachments) : undefined;
    const expectedPrompt = trimmed || 'Please review the attached workspace files.';
    const previousMessages = agentStore.getSnapshot().messages;
    await agentStore.steerActiveTurn(trimmed, attachmentDtos);
    if (!agentStore.getSnapshot().messages.some((message) => message.role === 'user' && message.content === expectedPrompt && !previousMessages.includes(message))) return;
    clearAcceptedDraft(revision);
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
    setAttachOpen(false);
    setRulesOpen(false);
  }, [gate.ready, text, attachments, clearAcceptedDraft]);

  const handleSubmit = useCallback(() => {
    if (inFlight) {
      if (!gate.ready) return;
      const trimmed = text.trim();
      if (trimmed || attachments.length > 0) {
        void handleQueue();
      } else {
        agentStore.stopActiveTurn();
      }
      return;
    }
    if (!gate.ready) return;
    const trimmed = text.trim();
    if (!trimmed && attachments.length === 0) return;
    const context = surface === 'main' ? activeEditorContext() ?? undefined : undefined;
    const revision = draftRef.current.revision;
    const accepted =
      attachments.length > 0
        ? agentStore.sendUserMessage(trimmed, false, context, toAttachmentDtos(attachments))
        : agentStore.sendUserMessage(trimmed, false, context);
    if (!accepted) return;
    clearAcceptedDraft(revision);
    setHistoryIndex(-1);
    setSavedDraft('');
    setActiveMenu(null);
    setAttachOpen(false);
    setRulesOpen(false);
  }, [inFlight, gate.ready, text, attachments, handleQueue, surface, clearAcceptedDraft]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Tab' && e.shiftKey) {
      e.preventDefault();
      const idx = MODE_OPTIONS.findIndex((m) => m.id === permissionMode);
      const next = MODE_OPTIONS[(idx + 1) % MODE_OPTIONS.length]!;
      agentStore.setPermissionMode(next.id);
      return;
    }

    if (e.key === 'Enter') {
      if (e.shiftKey) {
        // Shift+Enter inserts newline
        return;
      }
      e.preventDefault();
      if (e.ctrlKey || e.metaKey) {
        // Ctrl+Enter or Cmd+Enter: Steer immediately!
        if (text.trim()) {
          handleSteer();
        }
        return;
      }
      if (inFlight) {
        if (!gate.ready) return;
        if (text.trim() || attachments.length > 0) {
          void handleQueue();
        } else {
          agentStore.stopActiveTurn();
        }
      } else {
        handleSubmit();
      }
      return;
    }

    if (e.key === 'Escape') {
      if (activeMenu) {
        setActiveMenu(null);
        return;
      }
      if (inFlight) {
        e.preventDefault();
        agentStore.stopActiveTurn();
        return;
      }
    }

    // Up / Down arrow prompt history recall on single-line or empty input
    if (e.key === 'ArrowUp' && !text.includes('\n')) {
      if (promptHistory.length === 0) return;
      e.preventDefault();
      if (historyIndex === -1) {
        setSavedDraft(text);
        const nextIndex = 0;
        setHistoryIndex(nextIndex);
        setText(promptHistory[nextIndex] || '');
      } else if (historyIndex < promptHistory.length - 1) {
        const nextIndex = historyIndex + 1;
        setHistoryIndex(nextIndex);
        setText(promptHistory[nextIndex] || '');
      }
    } else if (e.key === 'ArrowDown' && !text.includes('\n')) {
      if (historyIndex > 0) {
        e.preventDefault();
        const nextIndex = historyIndex - 1;
        setHistoryIndex(nextIndex);
        setText(promptHistory[nextIndex] || '');
      } else if (historyIndex === 0) {
        e.preventDefault();
        setHistoryIndex(-1);
        setText(savedDraft);
      }
    }
  };

  const currentMode =
    MODE_OPTIONS.find((m) => m.id === permissionMode) || MODE_OPTIONS[0];
  const CurrentModeIcon = currentMode.icon;

  const resolvedModels: ModelItem[] = useMemo(() => {
    // Settings > Providers can disable whole providers. Keep the currently
    // selected model visible so a live session is never broken implicitly,
    // while hiding retired/unavailable models from active recommendations.
    const disabled = appSnapshot.bootstrap?.settings.disabled_providers ?? [];
    return availableModels
      .filter((model) => {
        if (model.id === selectedModelId) return true;
        if (!model.available) return false;
        const provider = providerIdForModel(model.id, model.display_name ?? '');
        return !provider || !disabled.includes(provider);
      })
      .map(describeModel);
  }, [availableModels, selectedModelId, appSnapshot.bootstrap?.settings.disabled_providers]);

  const matchedModel = resolvedModels.find(
    (m) => m.id === selectedModelId || m.name === selectedModelId,
  );

  const availableEffortOptions = useMemo(() => {
    const modes = matchedModel?.reasoningModes;
    if (!modes || modes.length === 0) return EFFORT_OPTIONS;
    const filtered = EFFORT_OPTIONS.filter((opt) => modes.includes(opt.id));
    return filtered.length > 0 ? filtered : EFFORT_OPTIONS;
  }, [matchedModel]);

  const currentEffort =
    availableEffortOptions.find((e) => e.id === reasoningEffort) ||
    EFFORT_OPTIONS.find((e) => e.id === reasoningEffort) ||
    availableEffortOptions[0] ||
    EFFORT_OPTIONS[2];

  const filteredModels = useMemo(() => {
    const q = modelSearch.trim().toLowerCase();
    if (!q) return resolvedModels;
    return resolvedModels.filter(
      (m) =>
        m.name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q) ||
        m.provider.toLowerCase().includes(q),
    );
  }, [resolvedModels, modelSearch]);

  const sendableModelId = resolveModelId(storeState);
  const displayModelName = matchedModel
    ? matchedModel.available
      ? matchedModel.name
      : `${matchedModel.name} (Unavailable)`
    : sendableModelId ??
      (selectedModelId.trim()
        ? `${selectedModelId.trim()} (Unavailable)`
        : availableModels.length === 0
        ? 'No model'
        : 'Select a model');

  const sendBlocked = !inFlight && !gate.ready;
  const wordCount = text.trim() ? text.trim().split(/\s+/).length : 0;
  const rulesInEffect = storeState.rulesInEffect;
  const activeRulesCount = rulesInEffect.filter((rule) => rule.active).length;

  return (
    <div
      ref={containerRef}
      data-composer-surface={surface}
      className={`agent-composer hero-composer-card ${isFocused ? 'is-focused' : ''} ${
        inFlight ? 'is-in-flight' : ''
      }${isDragOver ? ' is-drag-over' : ''}`}
      onDragOver={(event) => {
        if (!Array.from(event.dataTransfer.types).includes('Files')) return;
        event.preventDefault();
        setIsDragOver(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setIsDragOver(false);
      }}
      onDrop={handleDrop}
    >
      {/* 1. Attached workspace files. Paths only: the agent reads them. */}
      {attachments.length > 0 && (
        <div className="agent-attach-chips" role="list" aria-label="Attached files">
          {attachments.map((path) => (
            <span className="agent-attach-chip" role="listitem" key={path} title={path}>
              <FileText size={11} aria-hidden="true" />
              <button type="button" className="agent-attach-chip-name" aria-label={`Preview attachment ${baseName(path)}`}
                onClick={() => onPreviewAttachment ? onPreviewAttachment(path) : void editorStore.openPath(path, { preview: true })}>{baseName(path)}</button>
              <button
                type="button"
                className="agent-attach-chip-remove"
                aria-label={`Remove attachment ${baseName(path)}`}
                onClick={() => toggleAttachment(path)}
              >
                <X size={10} aria-hidden="true" />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* 3. Smooth Auto-Expanding Text Input Area */}
      <div className="composer-input-row">
        {!isFocused && !text && (
          <p
            className="agent-composer-placeholder"
            onClick={() => textareaRef.current?.focus()}
          >
            {gate.reason === 'no_workspace'
              ? 'Open a folder to let the agent read and write…'
              : gate.reason === 'no_model'
              ? 'Choose a provider to start…'
              : 'Ask anything…'}
          </p>
        )}
        <textarea
          ref={textareaRef}
          className="composer-textarea"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={handleKeyDown}
          onFocus={() => setIsFocused(true)}
          onBlur={() => setIsFocused(false)}
          onSelect={(event) => updateDraft({ selectionStart: event.currentTarget.selectionStart, selectionEnd: event.currentTarget.selectionEnd })}
          rows={1}
          aria-label="Composer prompt input"
          placeholder=""
        />
      </div>

      {/* 3. Blocked-send explanation (no workspace / no model) */}
      {!gate.ready && (
        <div
          className="composer-send-blocked"
          role="status"
          aria-live="polite"
        >
          <ShieldAlert size={12} className="text-amber-400" aria-hidden="true" />
          <span>{gate.message}</span>
        </div>
      )}

      {/* 4. Floating Dropdowns */}

      {/* Permission Mode Selector */}
      {activeMenu === 'mode' && (
        <div id="composer-mode-menu" className="composer-floating-menu mode-menu scale-in-menu" role="menu">
          <div className="floating-menu-header">Permission Mode</div>
          {MODE_OPTIONS.map((opt) => {
            const Icon = opt.icon;
            const isSelected = opt.id === permissionMode;
            return (
              <button
                key={opt.id}
                type="button"
                className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                role="menuitem"
                onClick={() => {
                  agentStore.setPermissionMode(opt.id);
                  setActiveMenu(null);
                }}
              >
                <div className={`menu-icon-wrap ${opt.colorClass}`}>
                  <Icon size={14} />
                </div>
                <div className="menu-item-text">
                  <div className="item-title-row">
                    <span className="item-title">{opt.label}</span>
                  </div>
                  <div className="item-desc">{opt.description}</div>
                </div>
                {isSelected && <Check size={13} className="menu-check text-ok" />}
              </button>
            );
          })}
        </div>
      )}

      {/* Model Selector Dropdown — backend catalog only */}
      {activeMenu === 'model' && (
        <div id="composer-model-menu" className="composer-floating-menu model-menu scale-in-menu" role="menu">
          <div className="floating-menu-header">Inference Model &amp; Provider</div>

          {availableModels.length === 0 ? (
            <div className="model-no-results" role="status">
              <div className="item-title">
                Choose a provider to start
              </div>
              <div className="item-desc">
                No models were reported by the backend. Choose a provider, then refresh
                the catalog.
              </div>
              <button
                type="button"
                className="floating-menu-item"
                role="menuitem"
                onClick={handleRefreshModels}
              >
                <Zap size={13} className="text-amber-400 flex-shrink-0" />
                <span className="item-title">Refresh models</span>
              </button>
              {modelError && (
                <div className="item-desc model-no-results-error">
                  {modelError}
                </div>
              )}
            </div>
          ) : (
            <>
              {/* Real-time Search Filter Input */}
              <div className="model-search-box">
                <Search size={13} className="model-search-icon" />
                <input
                  ref={modelSearchInputRef}
                  type="text"
                  className="model-search-input"
                  placeholder="Search models or providers..."
                  value={modelSearch}
                  onChange={(e) => setModelSearch(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key !== 'Escape') return;
                    e.preventDefault();
                    e.stopPropagation();
                    setActiveMenu(null);
                    modelTriggerRef.current?.focus();
                  }}
                  aria-label="Search models"
                />
                {modelSearch && (
                  <button
                    type="button"
                    className="model-search-clear"
                    onClick={() => setModelSearch('')}
                    title="Clear search"
                    aria-label="Clear search"
                  >
                    <X size={11} />
                  </button>
                )}
              </div>

              <div className="model-items-list">
                {filteredModels.length === 0 ? (
                  <div className="model-no-results">No matching models found</div>
                ) : (
                  filteredModels.map((m) => {
                    const isSelected =
                      selectedModelId === m.id ||
                      selectedModelId === m.name ||
                      sendableModelId === m.id;
                    const disabledForAgent =
                      !m.available || m.toolCapability === 'unsupported';
                    const capabilityTag = !m.available
                      ? 'Unavailable'
                      : m.toolCapability === 'unsupported'
                      ? 'No tools'
                      : m.toolCapability === 'unknown'
                      ? 'Unverified'
                      : null;
                    return (
                      <button
                        key={m.id}
                        type="button"
                        className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                        role="menuitem"
                        disabled={disabledForAgent}
                        title={
                          !m.available
                            ? 'Model is retired/unavailable and cannot be used.'
                            : m.toolCapability === 'unsupported'
                            ? 'Model does not support tool calling for agent turns.'
                            : m.toolCapability === 'unknown'
                            ? 'Tool support is unverified and will be validated before running agent tools.'
                            : undefined
                        }
                        onClick={() => {
                          if (disabledForAgent) return;
                          agentStore.setSelectedModel(m.id);
                          setActiveMenu(null);
                        }}
                      >
                        <Sparkles size={13} className="text-emerald-400 flex-shrink-0" />
                        <div className="menu-item-text">
                          <div className="item-title-row">
                            <span className="item-title">{m.name}</span>
                            <span className={`provider-badge ${m.colorClass}`}>
                              {m.provider}
                            </span>
                            {m.reasoningCapability === 'native' && (
                              <span className="provider-badge provider-custom">
                                Thinking
                              </span>
                            )}
                            {capabilityTag && (
                              <span className="provider-badge provider-custom">
                                {capabilityTag}
                              </span>
                            )}
                          </div>
                          {m.id !== m.name && <div className="item-desc">{m.id}</div>}
                        </div>
                        {isSelected && <Check size={13} className="menu-check text-ok" />}
                      </button>
                    );
                  })
                )}
              </div>
            </>
          )}

          <div className="menu-divider" />
          <button
            type="button"
            className="floating-menu-item model-settings-item"
            role="menuitem"
            onClick={() => {
              window.dispatchEvent(
                new CustomEvent(surface === 'main' ? 'skarn:open-settings' : 'skarn:open-connect'),
              );
              setActiveMenu(null);
            }}
          >
            <Key size={13} className="text-accent flex-shrink-0" />
            <div className="menu-item-text">
              <div className="item-title">Provider Settings…</div>
              <div className="item-desc">Configure providers and API keys</div>
            </div>
          </button>
        </div>
      )}

      {/* Reasoning Depth Dropdown */}
      {activeMenu === 'effort' && (
        <div id="composer-effort-menu" className="composer-floating-menu effort-menu scale-in-menu" role="menu">
          <div className="floating-menu-header">
            {matchedModel?.reasoningCapability === 'unsupported'
              ? 'Prompt Depth (No Native Thinking)'
              : matchedModel?.reasoningCapability === 'native'
              ? 'Native Thinking Mode'
              : 'Reasoning Depth'}
          </div>
          {availableEffortOptions.map((opt) => {
            const isSelected = opt.id === reasoningEffort;
            return (
              <button
                key={opt.id}
                type="button"
                className={`floating-menu-item ${isSelected ? 'is-selected' : ''}`}
                role="menuitem"
                onClick={() => {
                  agentStore.setReasoningEffort(opt.id);
                  setActiveMenu(null);
                }}
              >
                <Zap size={13} className="text-amber-400 flex-shrink-0" />
                <div className="menu-item-text">
                  <div className="item-title">{opt.label}</div>
                  <div className="item-desc">{opt.desc}</div>
                </div>
                {isSelected && <Check size={13} className="menu-check text-ok" />}
              </button>
            );
          })}
        </div>
      )}

      {/* 4b. Workspace file picker. Only real tree entries and open tabs. */}
      {attachOpen && (
        <div
          className="agent-attach-menu"
          role="dialog"
          aria-label="Attach workspace files"
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.stopPropagation();
              setAttachOpen(false);
            }
          }}
        >
          <div className="agent-attach-menu-head">
            <span className="agent-attach-menu-title">Attach workspace files</span>
            <span className="agent-attach-menu-count">
              {attachments.length}/{MAX_ATTACHMENTS}
            </span>
          </div>
          <input
            className="agent-attach-search"
            value={attachQuery}
            onChange={(event) => setAttachQuery(event.target.value)}
            placeholder="Search workspace files…"
            aria-label="Search workspace files"
            autoFocus
          />
          <div className="agent-attach-list" role="listbox" aria-label="Workspace files" aria-multiselectable="true">
            {workspaceFiles.length === 0 ? (
              <p className="agent-attach-empty">Open a folder to attach workspace files.</p>
            ) : filteredFiles.length === 0 ? (
              <p className="agent-attach-empty">No files match “{attachQuery.trim()}”.</p>
            ) : (
              filteredFiles.map((file) => {
                const selected = attachments.includes(file.path);
                const atLimit = !selected && attachments.length >= MAX_ATTACHMENTS;
                return (
                  <button
                    key={file.path}
                    type="button"
                    role="option"
                    aria-selected={selected}
                    className={`agent-attach-file${selected ? ' is-selected' : ''}`}
                    title={file.path}
                    disabled={atLimit}
                    onClick={() => toggleAttachment(file.path)}
                  >
                    <FileText size={12} className="agent-attach-file-icon" aria-hidden="true" />
                    <span className="agent-attach-file-name">{file.name}</span>
                    <span className="agent-attach-file-path">{file.path}</span>
                    {file.open && <span className="agent-attach-file-open">open</span>}
                    {selected && <Check size={12} className="agent-attach-file-check" aria-hidden="true" />}
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}

      {/* 4c. Rules in effect popover (active global/workspace vs untrusted-skipped) */}
      {rulesOpen && (
        <div
          id="composer-rules-popover"
          className="composer-rules-popover"
          role="dialog"
          aria-label="Rules in effect"
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.stopPropagation();
              setRulesOpen(false);
            }
          }}
        >
          <div className="composer-rules-header">
            <span className="composer-rules-title">Rules in effect ({activeRulesCount})</span>
            <span
              className={`composer-rules-trust-pill ${
                storeState.workspaceTrusted ? 'is-trusted' : 'is-untrusted'
              }`}
            >
              {storeState.workspaceTrusted ? 'Trusted workspace' : 'Untrusted workspace'}
            </span>
          </div>
          {rulesInEffect.length === 0 ? (
            <p className="composer-rules-empty">No AGENTS.md or .skarn/rules sources loaded.</p>
          ) : (
            <ul className="composer-rules-list">
              {rulesInEffect.map((rule) => (
                <li
                  key={`${rule.scope}:${rule.path}`}
                  className={`composer-rule-item ${rule.active ? 'is-active' : 'is-skipped'}`}
                >
                  <div className="composer-rule-row">
                    <span className="composer-rule-scope">{rule.scope}</span>
                    <span className="composer-rule-path" title={rule.path}>
                      {rule.path}
                    </span>
                    <span className={`composer-rule-status ${rule.active ? 'is-active' : 'is-skipped'}`}>
                      {rule.active
                        ? 'Active'
                        : rule.inactive_reason === 'workspace_untrusted' || !storeState.workspaceTrusted
                        ? 'Skipped (untrusted)'
                        : rule.inactive_reason ?? 'Skipped'}
                    </span>
                  </div>
                  {rule.digest && (
                    <span className="composer-rule-digest">{rule.digest.slice(0, 12)}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* 5. Bottom Toolbar */}
      <div className="composer-toolbar">
        <div className="toolbar-left">
          {/* Attach workspace files. Paths only, read by the agent's tools. */}
          <button
            type="button"
            className={`composer-pill attach-pill${attachOpen ? ' is-open' : ''}`}
            onClick={() => setAttachOpen((open) => !open)}
            disabled={workspaceFiles.length === 0}
            title={
              workspaceFiles.length === 0
                ? 'Open a folder to attach workspace files'
                : 'Attach workspace files for this prompt'
            }
            aria-label="Attach workspace files"
            aria-haspopup="dialog"
            aria-expanded={attachOpen}
          >
            <Paperclip size={13} />
            <span className="pill-text">Attach</span>
            {attachments.length > 0 && (
              <span className="agent-attach-count">{attachments.length}</span>
            )}
          </button>

          {/* Permission Mode pill (ZCode V4ComposerModeControls) */}
          <button
            type="button"
            className="composer-pill mode-pill"
            onClick={() => setActiveMenu((cur) => (cur === 'mode' ? null : 'mode'))}
            title="Toggle permission policy (Shift+Tab to cycle)"
            aria-haspopup="menu"
            aria-expanded={activeMenu === 'mode'}
            aria-controls={activeMenu === 'mode' ? 'composer-mode-menu' : undefined}
          >
            <CurrentModeIcon size={13} className={currentMode.colorClass} />
            <span className="pill-text">{currentMode.label}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* Rules in effect chip */}
          <button
            type="button"
            className={`composer-pill rules-pill${rulesOpen ? ' is-open' : ''}`}
            onClick={() => setRulesOpen((open) => !open)}
            title="Inspect loaded AGENTS.md and .skarn/rules sources"
            aria-haspopup="dialog"
            aria-expanded={rulesOpen}
            aria-controls={rulesOpen ? 'composer-rules-popover' : undefined}
          >
            <FileText size={12} />
            <span className="pill-text">Rules in effect ({activeRulesCount})</span>
          </button>

          {/* Prompt word count (measured, not estimated) */}
          {text.trim() && (
            <span className="composer-token-pill" title="Word count for this prompt">
              {wordCount}w
            </span>
          )}
        </div>

        <div className="toolbar-right">
          {/* Model Selector Pill with Provider Indicator */}
          <button
            ref={modelTriggerRef}
            type="button"
            className="composer-pill model-pill"
            title="Active Model: click to switch or search"
            onClick={() => setActiveMenu((cur) => (cur === 'model' ? null : 'model'))}
            aria-haspopup="menu"
            aria-expanded={activeMenu === 'model'}
            aria-controls={activeMenu === 'model' ? 'composer-model-menu' : undefined}
          >
            <span className="model-dot" />
            <span className="pill-text">{displayModelName}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* Reasoning Effort Pill */}
          <button
            type="button"
            className="composer-pill effort-pill"
            onClick={() => setActiveMenu((cur) => (cur === 'effort' ? null : 'effort'))}
            title="Reasoning effort"
            aria-haspopup="menu"
            aria-expanded={activeMenu === 'effort'}
            aria-controls={activeMenu === 'effort' ? 'composer-effort-menu' : undefined}
          >
            <Zap size={12} className="effort-icon" />
            <span className="pill-text">{currentEffort.label}</span>
            <ChevronDown size={11} className="pill-chevron" />
          </button>

          {/* In-Flight Actions vs Send/Stop Button */}
          {inFlight && (text.trim() || attachments.length > 0) ? (
            <div className="composer-inflight-actions-group">
              {/* Steer button (Ctrl+Enter) */}
              <button
                type="button"
                className="composer-action-pill composer-steer-btn"
                onClick={handleSteer}
                disabled={!gate.ready}
                title="Steer active run (Ctrl+Enter): inject urgent course correction directive"
                aria-label="Steer active run"
              >
                <DirectSend size={12} className="btn-icon" />
                <span className="pill-text">Steer</span>
              </button>

              {/* Queue button (Enter) */}
              <button
                type="button"
                className="composer-action-pill composer-queue-btn"
                onClick={handleQueue}
                disabled={!gate.ready}
                title="Queue prompt (Enter): runs automatically when active turn completes"
                aria-label="Queue prompt"
              >
                <Plus size={12} className="btn-icon" />
                <span className="pill-text">Queue</span>
              </button>

              {/* Stop button (Esc) */}
              <button
                type="button"
                className="composer-send-btn is-stop"
                onClick={() => agentStore.stopActiveTurn()}
                title="Stop execution (Esc)"
                aria-label="Stop"
              >
                <Square size={12} />
              </button>
            </div>
          ) : (
            /* Functional Glowing Green Send / Morphing Stop Action Button */
            <button
              type="button"
              className={`composer-send-btn ${inFlight ? 'is-stop' : ''} ${
                (text.trim() || attachments.length > 0) && gate.ready ? 'has-text' : ''
              } ${sendBlocked ? 'is-blocked' : ''}`}
              onClick={handleSubmit}
              disabled={sendBlocked}
              aria-disabled={sendBlocked}
              title={
                sendBlocked
                  ? gate.message ?? 'Sending is unavailable'
                  : inFlight
                  ? 'Stop execution (Esc)'
                  : 'Send prompt (Enter)'
              }
              aria-label={inFlight ? 'Stop' : 'Send'}
            >
              {inFlight ? <Square size={12} /> : <ArrowUp size={14} />}
            </button>
          )}
        </div>
      </div>

      {/* 6. VoiceBeam AI activity meter at the bottom edge */}
      <VoiceBeam active={inFlight} level={streamingActivityLevel} />
    </div>
  );
};
