// Appearance runtime tests: theme resolution, switching, persistence of the
// stored spelling, and reduced-motion independence.
import { describe, expect, it, vi } from 'vitest';
import {
  DEFAULT_THEME,
  THEME_OPTIONS,
  THEME_VALUES,
  applyAppearance,
  applyReducedMotion,
  applyFontScale,
  applyTheme,
  isThemeValue,
  normalizeThemeValue,
  resolveTheme,
  watchSystemTheme,
} from './theme';

function root(): HTMLElement {
  const element = document.createElement('div');
  document.body.append(element);
  return element;
}

describe('theme resolution', () => {
  it('accepts exactly the values Rust persists', () => {
    expect([...THEME_VALUES]).toEqual(['selora-dark', 'galaxy', 'dark', 'space-black', 'light', 'system']);
    expect(THEME_OPTIONS.map((option) => option.value)).toEqual(['selora-dark', 'galaxy', 'dark', 'light', 'system']);
    expect(THEME_OPTIONS.some((option) => /OLED/.test(option.label))).toBe(false);
    for (const value of THEME_VALUES) expect(isThemeValue(value)).toBe(true);
    expect(isThemeValue('not-a-theme')).toBe(false);
  });

  it('maps both dark aliases and space-black to the dark palette', () => {
    expect(resolveTheme('selora-dark')).toBe('dark');
    expect(resolveTheme('dark')).toBe('dark');
    expect(resolveTheme('space-black')).toBe('dark');
    expect(resolveTheme('light')).toBe('light');
    expect(resolveTheme(undefined)).toBe('dark');
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
  });

  it('tracks system preference changes without leaking listeners', () => {
    const element = root();
    const media = {
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    };
    vi.stubGlobal('matchMedia', vi.fn(() => media));
    const stop = watchSystemTheme('system', element);
    expect(media.addEventListener).toHaveBeenCalledWith('change', expect.any(Function));
    stop();
    expect(media.removeEventListener).toHaveBeenCalledWith('change', expect.any(Function));
    vi.unstubAllGlobals();
  });

  it('re-resolves the palette when the OS preference flips', () => {
    const element = root();
    const media = {
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    };
    vi.stubGlobal('matchMedia', vi.fn(() => media));
    const stop = watchSystemTheme('system', element);
    const listener = media.addEventListener.mock.calls[0][1] as () => void;

    listener();
    expect(element.dataset.theme).toBe('light');
    expect(element.dataset.themePreference).toBe('system');

    media.matches = true;
    listener();
    expect(element.dataset.theme).toBe('dark');
    expect(element.dataset.themePreference).toBe('system');

    stop();
    vi.unstubAllGlobals();
  });

  it('does not follow the OS while an explicit theme is selected', () => {
    const element = root();
    vi.stubGlobal('matchMedia', vi.fn());
    const stop = watchSystemTheme('light', element);
    expect(window.matchMedia).not.toHaveBeenCalled();
    expect(applyTheme('light', element)).toBe('light');
    expect(element.dataset.theme).toBe('light');
    stop();
    vi.unstubAllGlobals();
  });

  it('scales the root in coarse steps without scaling default text', () => {
    const element = root();
    applyFontScale(14, element);
    expect(element.dataset.fontScale).toBeUndefined();
    applyFontScale(18, element);
    expect(element.dataset.fontScale).toBe('large');
    applyFontScale(22, element);
    expect(element.dataset.fontScale).toBe('x-large');
  });

  it('clears a stale scale for unusable values', () => {
    const element = root();
    applyFontScale(24, element);
    expect(element.dataset.fontScale).toBe('x-large');
    applyFontScale(undefined, element);
    expect(element.dataset.fontScale).toBeUndefined();
    applyFontScale(Number.NaN, element);
    expect(element.dataset.fontScale).toBeUndefined();
  });

  it('applies the scale together with the other appearance settings', () => {
    const element = root();
    applyAppearance({ theme: 'dark', reduced_motion: false, editor_font_size: 18 }, element);
    expect(element.dataset.fontScale).toBe('large');
    applyAppearance({ theme: 'light', reduced_motion: true, editor_font_size: 14 }, element);
    expect(element.dataset.fontScale).toBeUndefined();
    expect(element.dataset.theme).toBe('light');
    expect(element.dataset.reducedMotion).toBe('true');
  });

  it('keeps an unknown stored value out of the select and falls back safely', () => {
    expect(normalizeThemeValue('light')).toBe('light');
    expect(normalizeThemeValue('unknown')).toBe(DEFAULT_THEME);
    expect(normalizeThemeValue(null)).toBe(DEFAULT_THEME);
  });
});

describe('theme switching', () => {
  it('writes data-theme and the selected palette on the root', () => {
    const element = root();
    expect(applyTheme('light', element)).toBe('light');
    expect(element.dataset.theme).toBe('light');
    expect(applyTheme('space-black', element)).toBe('dark');
    expect(element.dataset.theme).toBe('dark');
    expect(applyTheme('selora-dark', element)).toBe('dark');
  });

  it('applies theme and reduced motion together without coupling them', () => {
    const element = root();

    applyAppearance({ theme: 'light', reduced_motion: true }, element);
    expect(element.dataset.theme).toBe('light');
    expect(element.dataset.reducedMotion).toBe('true');

    // Switching the palette must not clear the motion preference.
    applyTheme('dark', element);
    expect(element.dataset.theme).toBe('dark');
    expect(element.dataset.reducedMotion).toBe('true');

    applyReducedMotion(false, element);
    expect(element.dataset.reducedMotion).toBeUndefined();
    expect(element.dataset.theme).toBe('dark');

    applyAppearance({ theme: 'light', reduced_motion: false }, element);
    expect(element.dataset.theme).toBe('light');
    expect(element.dataset.reducedMotion).toBeUndefined();
  });

  it('defaults the document root to dark before any bootstrap lands', () => {
    // The app never writes data-theme until Rust reports settings; the CSS
    // base palette is the dark ramp, so a root without the attribute is dark.
    expect(resolveTheme(undefined)).toBe('dark');
  });
});
