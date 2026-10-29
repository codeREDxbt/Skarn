// Selora IDE — appearance (theme + reduced motion) applied at runtime.
//
// Rust owns the persisted values (`ui.theme`, `ui.reduce_motion`); this module
// maps them onto the document root. A theme is a token remap, never a CSS
// filter: `data-theme="light"` swaps the palette in palette.css and
// `data-theme="dark"` keeps the default ink ramp.
export const THEME_VALUES = ['selora-dark', 'galaxy', 'dark', 'space-black', 'light', 'system'] as const;

export type ThemeValue = (typeof THEME_VALUES)[number];
export type ResolvedTheme = 'dark' | 'light' | 'galaxy';

export const DEFAULT_THEME: ThemeValue = 'selora-dark';

export const THEME_OPTIONS: ReadonlyArray<{ value: ThemeValue; label: string }> = [
  { value: 'selora-dark', label: 'Selora Cosmic Dark' },
  { value: 'galaxy', label: 'Selora Galaxy' },
  { value: 'dark', label: 'Dark (alias)' },
  { value: 'light', label: 'Selora Moonlight (Light)' },
  { value: 'system', label: 'System' },
];

/** True when Rust's stored spelling is one the backend accepts. */
export function isThemeValue(value: string): value is ThemeValue {
  return (THEME_VALUES as readonly string[]).includes(value);
}

/** Keep a stored accepted value, otherwise fall back to the documented default. */
export function normalizeThemeValue(value: string | null | undefined): ThemeValue {
  return value && isThemeValue(value) ? value : DEFAULT_THEME;
}

/**
 * The OS preference, dark when the platform cannot tell us. Called only when
 * the stored value is `system`: an explicit theme never reads the media query.
 */
function prefersDarkScheme(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return true;
  try {
    return window.matchMedia('(prefers-color-scheme: dark)')?.matches ?? true;
  } catch {
    return true;
  }
}

/**
 * The two real palettes. `dark`, `selora-dark` and `space-black` are aliases
 * for the ink ramp; `light` is Selora Moonlight.
 */
export function resolveTheme(
  value: string | null | undefined,
  prefersDark?: boolean,
): ResolvedTheme {
  if (value === 'light') return 'light';
  if (value === 'galaxy') return 'galaxy';
  if (value === 'system') return (prefersDark ?? prefersDarkScheme()) ? 'dark' : 'light';
  return 'dark';
}

/** Apply a stored theme value to the document root and return the palette it selected. */
export function applyTheme(
  value: string | null | undefined,
  root: HTMLElement = document.documentElement,
): ResolvedTheme {
  const resolved = resolveTheme(value);
  root.dataset.theme = resolved;
  root.dataset.themePreference = normalizeThemeValue(value);
  return resolved;
}

/** Keep system theme in sync when the OS color preference changes. */
export function watchSystemTheme(
  value: string | null | undefined,
  root: HTMLElement = document.documentElement,
): () => void {
  if (value !== 'system' || typeof window === 'undefined' || typeof window.matchMedia !== 'function') return () => undefined;
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  if (!media || typeof media.addEventListener !== 'function') return () => undefined;
  const update = () => applyTheme('system', root);
  media.addEventListener('change', update);
  return () => media.removeEventListener?.('change', update);
}

/** Map the persisted editor size to calm, coarse UI accessibility scale steps. */
export function applyFontScale(
  fontSize: number | null | undefined,
  root: HTMLElement = document.documentElement,
): void {
  const size = Number(fontSize);
  if (!Number.isFinite(size) || size <= 14) delete root.dataset.fontScale;
  else if (size >= 22) root.dataset.fontScale = 'x-large';
  else root.dataset.fontScale = 'large';
}

/**
 * DESIGN_TOKENS §7.4: the in-app flag is `<html data-reduced-motion="true">`.
 * It only freezes loops (`--motion-loops: 0`); it never rewrites durations.
 */
export function applyReducedMotion(
  reduced: boolean,
  root: HTMLElement = document.documentElement,
): void {
  if (reduced) root.dataset.reducedMotion = 'true';
  else delete root.dataset.reducedMotion;
}

/** Apply both appearance settings from a bootstrap `SettingsView`. */
export function applyAppearance(
  settings: { theme: string; reduced_motion: boolean; editor_font_size?: number },
  root: HTMLElement = document.documentElement,
): ResolvedTheme {
  const resolved = applyTheme(settings.theme, root);
  applyReducedMotion(settings.reduced_motion, root);
  applyFontScale(settings.editor_font_size, root);
  return resolved;
}
