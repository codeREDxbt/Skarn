// Selora IDE — persisted UI sound preference.
//
// A tiny external store in the shape React's useSyncExternalStore expects:
// `subscribe` / `getSnapshot`, plus `setEnabled` and a `playCue` wrapper that
// is silent while the preference is OFF (the default). The preference lives
// in localStorage under `selora:ui-sounds`; the audio itself stays in
// `sound.ts`, which owns the global mute flag and the AudioContext.
import {
  playCue as playEngineCue,
  setSoundEnabled as setEngineSoundEnabled,
  type SoundCue,
} from './sound';

export type { SoundCue };

export const SOUND_PREFERENCE_KEY = 'selora:ui-sounds';

type Listener = () => void;
const listeners = new Set<Listener>();

/** Default OFF: a fresh install is silent until the user opts in. */
function readPreference(): boolean {
  try {
    if (typeof localStorage === 'undefined') return false;
    return localStorage.getItem(SOUND_PREFERENCE_KEY) === 'true';
  } catch {
    return false;
  }
}

function writePreference(enabled: boolean): void {
  try {
    if (typeof localStorage === 'undefined') return;
    if (enabled) localStorage.setItem(SOUND_PREFERENCE_KEY, 'true');
    else localStorage.removeItem(SOUND_PREFERENCE_KEY);
  } catch {
    // Storage can be blocked; the in-memory preference still applies.
  }
}

let enabled = readPreference();
setEngineSoundEnabled(enabled);
let snapshot = enabled;

function notify(): void {
  snapshot = enabled;
  for (const listener of listeners) listener();
}

export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getSnapshot(): boolean {
  return snapshot;
}

/** Persist and apply the preference; notifies only on a real change. */
export function setEnabled(next: boolean): void {
  const value = Boolean(next);
  if (value === enabled) return;
  enabled = value;
  writePreference(value);
  setEngineSoundEnabled(value);
  notify();
}

/** Play a cue only while sounds are enabled; silent no-op otherwise. */
export function playCue(cue: SoundCue): void {
  if (!enabled) return;
  playEngineCue(cue);
}