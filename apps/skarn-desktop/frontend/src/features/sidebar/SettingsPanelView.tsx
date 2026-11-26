// Selora IDE — Interactive Settings Panel (DESIGN §3.3)
import { useEffect, useState, useMemo, useSyncExternalStore, type FC } from 'react';
import { localEditorPreferences } from '../editor/local-preferences';
import {
  updateSettings,
  restartLsp,
  type BootstrapState,
  type EnvelopeContext,
  type SettingsPatch,
} from '../../bridge/bridge';
import {
  Key,
  Code2,
  Eye,
  Terminal,
  Search,
} from '../../components/icons';
import { buildCommandList, commandCategory, validateKeymapConflicts } from '../keymap/commands';
import { canonicalKeybind, parseKeybind } from '../keymap/keybind';
import { THEME_OPTIONS, normalizeThemeValue } from '../theme/theme';
import { toastStore } from '../toasts/toast-store';
import { getSnapshot as getSoundSnapshot, setEnabled as setSoundEnabled, subscribe as subscribeSound } from '../sound/sound-store';
import { PROVIDER_PRESETS } from '../auth/ConnectModal';
import {
  activeProviderId,
  isProviderDisabled,
  providerIdForModel,
  withProviderEnabled,
} from '../auth/provider-visibility';

export interface SettingsPanelViewProps {
  bootstrap: BootstrapState;
  onOpenConnect?: () => void;
}

type SettingsTab = 'providers' | 'editor' | 'appearance' | 'keybindings';

interface KeybindingEntry {
  id: string;
  command: string;
  keys: string;
  category: string;
}

/**
 * The select offers the palettes users can choose. Rust still accepts the
 * legacy `space-black` spelling but resolves it to dark; a stored alias is not
 * silently rewritten in the display.
 */

/** Mirrors selora-settings' EDITOR_* range constants. */
const FONT_SIZE_MIN = 8;
const FONT_SIZE_MAX = 32;
const TAB_SIZE_MIN = 1;
const TAB_SIZE_MAX = 8;

type KeybindDraftKey =
  | 'keybind_command_palette'
  | 'keybind_fuzzy_find'
  | 'keybind_toggle_chat'
  | 'keybind_save';

const KEYBIND_FIELDS: Array<{ key: KeybindDraftKey; label: string; command: string }> = [
  { key: 'keybind_save', label: 'Save', command: 'Save' },
  { key: 'keybind_fuzzy_find', label: 'Find in Files', command: 'Find in Files' },
  { key: 'keybind_command_palette', label: 'Command Palette', command: 'Open Command Palette' },
  { key: 'keybind_toggle_chat', label: 'Toggle Agent Dock', command: 'Toggle Agent Dock' },
];

function formatError(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

export const SettingsPanelView: FC<SettingsPanelViewProps> = ({ bootstrap, onOpenConnect }) => {
  const localPreferences = useSyncExternalStore(localEditorPreferences.subscribe, localEditorPreferences.getSnapshot);
  const settings = bootstrap.settings;
  const isConnected = bootstrap.connection === 'connected';

  const [activeTab, setActiveTab] = useState<SettingsTab>('providers');
  const [keybindQuery, setKeybindQuery] = useState('');
  const [saving, setSaving] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [fontSizeDraft, setFontSizeDraft] = useState(String(settings.editor_font_size));
  const [tabSizeDraft, setTabSizeDraft] = useState(String(settings.editor_tab_size));
  const [keybindDrafts, setKeybindDrafts] = useState<Record<KeybindDraftKey, string>>({
    keybind_command_palette: settings.keybind_command_palette,
    keybind_fuzzy_find: settings.keybind_fuzzy_find,
    keybind_toggle_chat: settings.keybind_toggle_chat,
    keybind_save: settings.keybind_save,
  });

  // The backend event is the source of truth: after a save (or an external
  // settings change) the fields show the persisted value, not a stale draft.
  useEffect(() => {
    setFontSizeDraft(String(settings.editor_font_size));
    setTabSizeDraft(String(settings.editor_tab_size));
    setKeybindDrafts({
      keybind_command_palette: settings.keybind_command_palette,
      keybind_fuzzy_find: settings.keybind_fuzzy_find,
      keybind_toggle_chat: settings.keybind_toggle_chat,
      keybind_save: settings.keybind_save,
    });
  }, [
    settings.editor_font_size,
    settings.editor_tab_size,
    settings.keybind_command_palette,
    settings.keybind_fuzzy_find,
    settings.keybind_toggle_chat,
    settings.keybind_save,
  ]);

  // settings_update is not session-scoped: Rust dispatch rejects any
  // envelope `session_id` on it, so settings always send null.
  const context: EnvelopeContext = {
    core_instance_id: bootstrap.core_instance_id,
    workspace_id: bootstrap.workspace.id,
    session_id: null,
  };

  const persist = (patch: SettingsPatch): void => {
    setSettingsError(null);
    setFieldError(null);
    setSaving(true);
    void updateSettings(patch, context)
      .catch((cause: unknown) => setSettingsError(formatError(cause)))
      .finally(() => setSaving(false));
  };

  // Settings > Providers "Assistant providers". The list is the persisted
  // gate: a disabled provider is hidden from provider/model selection, but an
  // already-active provider is never switched or disconnected here.
  const disabledProviders = settings.disabled_providers ?? [];
  const catalogModels = bootstrap.models ?? [];
  const activeProvider = activeProviderId(bootstrap.active_model_id, catalogModels);
  const activeProviderPreset =
    PROVIDER_PRESETS.find((preset) => preset.id === activeProvider) ?? null;
  const activeProviderDisabled =
    activeProvider !== null && isProviderDisabled(disabledProviders, activeProvider);
  const activeModelLabel =
    catalogModels.find((model) => model.id === bootstrap.active_model_id)?.display_name ??
    bootstrap.active_model_id ??
    'none';
  const toggleProvider = (providerId: string, enabled: boolean): void => {
    persist({ disabled_providers: withProviderEnabled(disabledProviders, providerId, enabled) });
  };

const restartServer = (language: string): void => {
    setSettingsError(null);
    void restartLsp(language, context).catch((cause: unknown) =>
      setSettingsError(formatError(cause)),
    );
  };

  const lspServers = bootstrap.lsp_servers ?? [];

  const commitFontSize = (): void => {
    const value = Number(fontSizeDraft);
    if (!Number.isInteger(value) || value < FONT_SIZE_MIN || value > FONT_SIZE_MAX) {
      setFieldError(`Font size must be a whole number from ${FONT_SIZE_MIN} to ${FONT_SIZE_MAX}.`);
      return;
    }
    if (value === settings.editor_font_size) return;
    persist({ editor_font_size: value });
  };

  const commitKeybind = (key: KeybindDraftKey): void => {
    const label = KEYBIND_FIELDS.find((field) => field.key === key)?.label ?? key;
    const raw = keybindDrafts[key].trim();
    const parsed = parseKeybind(raw);
    if (!parsed) {
      setFieldError(`${label} must be one chord such as Ctrl+S. "${raw}" is not valid.`);
      return;
    }
    const canonical = canonicalKeybind(parsed);
    if (canonical === settings[key]) return;
    const conflicts = validateKeymapConflicts({ ...settings, [key]: canonical });
    if (conflicts.length > 0) {
      const conflict = conflicts[0];
      const labels = conflict.commands.map((command) => command.title).join(' and ');
      const message = `Shortcut conflict: ${conflict.shortcut} is assigned to ${labels}.`;
      setFieldError(message);
      toastStore.pushLocal('warning', message);
      return;
    }
    persist({ [key]: canonical } as SettingsPatch);
  };

  // The cheat sheet is built from the real command registry, so it can never
  // advertise a command the shell does not dispatch. Commands without a
  // shortcut are listed with "—" instead of an invented chord.
  const keybindingList: KeybindingEntry[] = useMemo(() => {
    return buildCommandList(settings).map((command) => ({
      id: command.id,
      command: command.title,
      keys: command.shortcut ?? '—',
      category: commandCategory(command.id),
    }));
  }, [settings]);

  const filteredKeybindings = useMemo(() => {
    if (!keybindQuery.trim()) return keybindingList;
    const q = keybindQuery.toLowerCase();
    return keybindingList.filter(
      (k) =>
        k.command.toLowerCase().includes(q) ||
        k.keys.toLowerCase().includes(q) ||
        k.category.toLowerCase().includes(q)
    );
  }, [keybindingList, keybindQuery]);

  const themeValue = normalizeThemeValue(settings.theme);
  const soundsEnabled = useSyncExternalStore(subscribeSound, getSoundSnapshot);

  return (
    <div className="sidebar-settings-panel">
      {/* Settings Navigation Tabs */}
      <nav className="settings-nav-tabs" aria-label="Settings Categories">
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'providers' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('providers')}
          title="Providers & Keys"
        >
          <Key size={12} />
          <span>Providers</span>
        </button>
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'editor' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('editor')}
          title="Editor Preferences"
        >
          <Code2 size={12} />
          <span>Editor</span>
        </button>
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'appearance' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('appearance')}
          title="Appearance & Theme"
        >
          <Eye size={12} />
          <span>Theme</span>
        </button>
        <button
          type="button"
          className={`settings-tab-btn ${activeTab === 'keybindings' ? 'is-active' : ''}`}
          onClick={() => setActiveTab('keybindings')}
          title="Keyboard Shortcuts"
        >
          <Terminal size={12} />
          <span>Keymap</span>
        </button>
      </nav>

      {settingsError ? (
        <p className="settings-card-hint" role="alert">
          Settings update failed: {settingsError}
        </p>
      ) : null}
      {fieldError ? (
        <p className="settings-card-hint" role="alert">
          {fieldError}
        </p>
      ) : null}

      {/* Tab 1: Providers */}
      {activeTab === 'providers' && (
        <div className="settings-section-content">
          {/* Status & Provider Card */}
          <div className="settings-card ai-gateway-card">
            <div className="settings-card-header">
              <span className="settings-card-title">AI Provider & Key</span>
              <span
                className={`settings-status-pill ${
                  isConnected ? 'is-connected' : 'is-disconnected'
                }`}
              >
                {isConnected ? 'Connected' : 'Not Connected'}
              </span>
            </div>
            <p className="settings-card-hint">
              Connect Anthropic, OpenAI, Local Ollama, DeepSeek, OpenRouter, GLM, or custom endpoints.
            </p>
            <button
              type="button"
              className="settings-primary-btn"
              onClick={onOpenConnect}
            >
              {isConnected ? 'Manage API Key & Provider' : 'Configure API Key'}
            </button>
          </div>

          {/* Assistant providers — persisted enable/disable gate. Visibility
              only: the stored key, live model and network state are untouched. */}
          <div className="settings-card" aria-labelledby="settings-assistant-providers-title">
            <div className="settings-card-header">
              <span className="settings-card-title" id="settings-assistant-providers-title">
                Assistant providers
              </span>
              <span className="settings-card-hint">
                {PROVIDER_PRESETS.length - disabledProviders.length} of {PROVIDER_PRESETS.length} enabled
              </span>
            </div>
            <p className="settings-card-hint">
              Turn a provider off to hide it from provider and model selection. Disabling
              keeps the stored key, does not switch the current model, and makes no network
              request.
            </p>
            {activeProviderDisabled && activeProviderPreset ? (
              <p className="settings-card-hint" role="alert" style={{ color: 'var(--c-fbbf24)' }}>
                {activeProviderPreset.name} is disabled but remains active for this session.
                Current model {activeModelLabel} is unchanged. Choose another model to stop
                using it.
              </p>
            ) : null}
            <ul className="settings-provider-list">
              {PROVIDER_PRESETS.map((preset) => {
                const enabled = !isProviderDisabled(disabledProviders, preset.id);
                const detected = catalogModels.some(
                  (model) => providerIdForModel(model.id, model.display_name) === preset.id,
                );
                const isActive = activeProvider === preset.id;
                return (
                  <li key={preset.id} className="settings-provider-item">
                    <div
                      style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: 2,
                        flex: 1,
                        minWidth: 0,
                      }}
                    >
                      <span
                        className="settings-label"
                        style={{ flex: 'none', display: 'flex', alignItems: 'center', gap: 6 }}
                      >
                        {preset.name}
                        {isActive ? (
                          <span className="settings-status-pill is-connected">Active</span>
                        ) : null}
                      </span>
                      <span className="settings-card-hint">
                        {!enabled
                          ? 'Disabled'
                          : detected
                            ? 'Reported in the current model catalog'
                            : `${preset.tag} · not in the current catalog`}
                      </span>
                    </div>
                    <label
                      className="settings-switch"
                      title={`${enabled ? 'Disable' : 'Enable'} ${preset.name}`}
                    >
                      <input
                        type="checkbox"
                        checked={enabled}
                        disabled={saving}
                        aria-label={`Enable ${preset.name}`}
                        onChange={(event) => toggleProvider(preset.id, event.target.checked)}
                      />
                      <span className="switch-slider" />
                    </label>
                  </li>
                );
              })}
            </ul>
          </div>

          {/* Agent workspace trust: risky shell commands need both this
              grant and an approval. Allowlisted read-only commands still
              run without it. */}
          <div className="settings-card" data-testid="workspace-trust-card">
            <div className="settings-card-header">
              <span className="settings-card-title">Workspace Trust</span>
              <span
                className={`settings-status-pill ${
                  settings.workspace_trusted ? 'is-connected' : 'is-disconnected'
                }`}
              >
                {settings.workspace_trusted ? 'Trusted' : 'Restricted'}
              </span>
            </div>
            <p className="settings-card-hint">
              Untrusted workspaces never run risky shell commands, even after
              you approve a command card. Read-only commands on the agent
              allowlist still run. Trust changes apply when Skarn restarts.
            </p>
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-workspace-trusted">
                Trust this workspace for risky agent commands
              </label>
              <label className="settings-switch">
                <input
                  id="settings-workspace-trusted"
                  type="checkbox"
                  checked={settings.workspace_trusted ?? false}
                  onChange={(e) => persist({ workspace_trusted: e.target.checked })}
                  aria-describedby="settings-workspace-trusted-hint"
                />
                <span className="switch-slider" />
              </label>
            </div>
            <span id="settings-workspace-trusted-hint" className="settings-card-hint">
              Restart Skarn after changing this setting; it is read at startup, and
              agent turns pause until the restart.
            </span>
          </div>
        </div>
      )}

      {/* Tab 2: Editor */}
      {activeTab === 'editor' && (
        <div className="settings-section-content">
          <p className="settings-card-hint">These options are stored locally. Session restoration saves file paths and cursor positions, not unsaved contents.</p>
          <div className="settings-group">
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-restore-session">Restore open files for this workspace</label>
              <input id="settings-restore-session" type="checkbox" checked={localPreferences.restoreSession} onChange={(e) => localEditorPreferences.update({ restoreSession: e.target.checked })} />
            </div>
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-auto-save">Auto Save</label>
              <input id="settings-auto-save" type="checkbox" checked={localPreferences.autoSave} onChange={(e) => localEditorPreferences.update({ autoSave: e.target.checked })} />
            </div>
          </div>
          <div className="settings-card">
            <div className="settings-card-header">
              <span className="settings-card-title">Language Servers</span>
              <span className="settings-card-hint">
                {lspServers.length === 0 ? 'Not started' : String(lspServers.length) + ' tracked'}
              </span>
            </div>
            <p className="settings-card-hint">
              Servers start when a file of their language opens. Restart re-runs the
              server for this workspace; paths, arguments and the master switch come
              from the <code>lsp</code> section of settings.json.
            </p>
            {lspServers.length === 0 ? (
              <p className="settings-card-hint" role="status">
                No language server has started in this workspace yet.
              </p>
            ) : (
              <div className="settings-group">
                {lspServers.map((server) => (
                  <div className="settings-row" key={server.language}>
                    <span className="settings-label">
                      {server.server}
                      <span
                        className="settings-card-hint"
                        data-testid={`lsp-state-${server.language}`}
                        role="status"
                      >
                        {' '}
                        — {server.state}
                        {server.error ? `: ${server.error}` : ''}
                      </span>
                    </span>
                    <button
                      type="button"
                      className="settings-primary-btn"
                      onClick={() => restartServer(server.language)}
                      aria-label={`Restart ${server.server}`}
                    >
                      Restart
                    </button>
                  </div>
                ))}
              </div>
            )}
          </div>
          <p className="settings-card-hint">
            These values save to settings.json and apply to the editor immediately.
          </p>
          <div className="settings-group">
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-font-size">
                Font Size (px)
              </label>
              <input
                id="settings-font-size"
                className="settings-number-input"
                type="number"
                min={FONT_SIZE_MIN}
                max={FONT_SIZE_MAX}
                step={1}
                value={fontSizeDraft}
                disabled={saving}
                aria-busy={saving}
                onChange={(e) => setFontSizeDraft(e.target.value)}
                onBlur={commitFontSize}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') commitFontSize();
                }}
                title={`Editor font size, ${FONT_SIZE_MIN}–${FONT_SIZE_MAX} px.`}
              />
            </div>

            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-tab-size">
                Tab Size
              </label>
              <select
                id="settings-tab-size"
                className="settings-select"
                value={tabSizeDraft}
                disabled={saving}
                onChange={(e) => {
                  const value = Number(e.target.value);
                  setTabSizeDraft(e.target.value);
                  if (value !== settings.editor_tab_size) persist({ editor_tab_size: value });
                }}
              >
                {Array.from(
                  { length: TAB_SIZE_MAX - TAB_SIZE_MIN + 1 },
                  (_, index) => TAB_SIZE_MIN + index,
                ).map((size) => (
                  <option key={size} value={size}>
                    {size} {size === 1 ? 'space' : 'spaces'}
                  </option>
                ))}
              </select>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-word-wrap-label">
                Word Wrap
              </span>
              <label className="settings-switch">
                <input
                  type="checkbox"
                  checked={settings.editor_word_wrap}
                  disabled={saving}
                  aria-busy={saving}
                  aria-labelledby="settings-word-wrap-label"
                  onChange={(e) => persist({ editor_word_wrap: e.target.checked })}
                />
                <span className="switch-slider" />
              </label>
            </div>
          </div>
        </div>
      )}

      {/* Tab 3: Appearance & Theme */}
      {activeTab === 'appearance' && (
        <div className="settings-section-content">
          <div className="settings-group">
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-theme-select">
                Color Theme
              </label>
              <select
                id="settings-theme-select"
                className="settings-select"
                aria-describedby="settings-theme-hint"
                value={themeValue}
                onChange={(e) => persist({ theme: e.target.value })}
              >
                {THEME_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <span id="settings-theme-hint" className="settings-card-hint">
                System follows your operating system appearance.
              </span>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-reduced-motion-label">
                Reduced Motion
              </span>
              <label className="settings-switch">
                <input
                  type="checkbox"
                  checked={settings.reduced_motion}
                  onChange={(e) => persist({ reduced_motion: e.target.checked })}
                  aria-labelledby="settings-reduced-motion-label"
                  aria-busy={saving}
                />
                <span className="switch-slider" />
              </label>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-sounds-label">
                Interface Sounds
              </span>
              <label
                className="settings-switch"
                title="Short synthesized cues for send, completion, errors and notifications. Off by default; stored on this device."
              >
                <input
                  type="checkbox"
                  checked={soundsEnabled}
                  onChange={(e) => setSoundEnabled(e.target.checked)}
                  aria-labelledby="settings-sounds-label"
                />
                <span className="switch-slider" />
              </label>
            </div>

            <div className="settings-row">
              <span className="settings-label" id="settings-check-updates-label">
                Automatic Updates
              </span>
              <label
                className="settings-switch"
                title="Check GitHub Releases once per launch and offer the newer installer in a centered prompt. The About dialog's manual check always works."
              >
                <input
                  type="checkbox"
                  checked={settings.check_for_updates ?? true}
                  disabled={saving}
                  aria-busy={saving}
                  aria-labelledby="settings-check-updates-label"
                  onChange={(e) => persist({ check_for_updates: e.target.checked })}
                />
                <span className="switch-slider" />
              </label>
            </div>
          </div>
        </div>
      )}

      {/* Tab 4: Keybindings */}
      {activeTab === 'keybindings' && (
        <div className="settings-section-content keybindings-section">
          <p className="settings-card-hint">
            Edit a chord, then press Enter or leave the field. Invalid chords are not saved.
          </p>
          <div className="settings-group">
            {KEYBIND_FIELDS.map((field) => (
              <div className="settings-row" key={field.key}>
                <label
                  className="settings-label"
                  htmlFor={`settings-keybind-${field.key}`}
                >
                  {field.label}
                </label>
                <input
                  id={`settings-keybind-${field.key}`}
                  className="settings-text-input"
                  type="text"
                  value={keybindDrafts[field.key]}
                  disabled={saving}
                  aria-busy={saving}
                  spellCheck={false}
                  onChange={(e) =>
                    setKeybindDrafts((current) => ({
                      ...current,
                      [field.key]: e.target.value,
                    }))
                  }
                  onBlur={() => commitKeybind(field.key)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') commitKeybind(field.key);
                  }}
                />
              </div>
            ))}
          </div>

          {/* Filter Input */}
          <div className="keybind-search-wrapper">
            <Search size={12} className="keybind-search-icon" />
            <input
              className="keybind-search-input"
              type="text"
              placeholder="Search keybindings (e.g. palette, agent, Ctrl)..."
              value={keybindQuery}
              onChange={(e) => setKeybindQuery(e.target.value)}
              aria-label="Filter keybindings"
            />
          </div>

          {/* Keybindings Cheat-Sheet Table */}
          <div className="keybindings-table-wrapper">
            <table className="keybindings-table">
              <thead>
                <tr>
                  <th>Command</th>
                  <th>Keybinding</th>
                </tr>
              </thead>
              <tbody>
                {filteredKeybindings.map((item) => (
                  <tr key={item.id} className="keybinding-row">
                    <td className="keybinding-cmd-cell">
                      <span className="keybinding-cmd-name">{item.command}</span>
                      <span className="keybinding-category">{item.category}</span>
                    </td>
                    <td className="keybinding-key-cell">
                      <kbd className="settings-kbd">{item.keys}</kbd>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Quick Settings & Keybind Echo (Required for Z2 spec parity & ShellGrid.test.tsx) */}
      {activeTab !== 'keybindings' && (
        <dl className="settings-echo">
          <div className="settings-echo-row">
            <dt>Command palette</dt>
            <dd>{settings.keybind_command_palette}</dd>
          </div>
          <div className="settings-echo-row">
            <dt>Toggle agent dock</dt>
            <dd>{settings.keybind_toggle_chat}</dd>
          </div>
          <div className="settings-echo-row">
            <dt>Save</dt>
            <dd>{settings.keybind_save}</dd>
          </div>
        </dl>
      )}

      <div className="settings-footer-info">
        <p className="settings-later">
          Theme, reduced motion, backend editor preferences, and the four keybinds save automatically
          through Rust.
        </p>
      </div>
    </div>
  );
};
