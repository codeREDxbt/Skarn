// Selora IDE — Galaxy palette identity.
//
// The Galaxy theme is a CSS-only remap: `data-theme="galaxy"` selects the
// selector in styles/theme-galaxy.css. This module owns the one spelling so a
// future settings entry, a settings-panel option and the test suite cannot
// drift apart. Persistence and runtime wiring land separately; nothing here
// mutates the document.
export const GALAXY_THEME = 'galaxy';

/** True when a stored/selected value names the Galaxy palette. */
export function isGalaxyTheme(value: string): boolean {
  return value.trim().toLowerCase() === GALAXY_THEME;
}