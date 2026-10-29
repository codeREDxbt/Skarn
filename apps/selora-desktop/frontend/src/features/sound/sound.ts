// Selora IDE — synthesized UI sound engine.
//
// Every cue is generated with the Web Audio API: no audio files, no network
// access. Construction of the AudioContext is deferred until the first cue is
// played from a user gesture (or the first gesture after sounds are enabled),
// so importing this module is inert and autoplay policy is respected.
//
// The engine is a leaf module: it never throws, never blocks, and stays quiet
// unless explicitly enabled. `sound-store.ts` owns the persisted preference
// and calls through to `setSoundEnabled` / `playCue`.

export type SoundCue = 'send' | 'complete' | 'error' | 'notify';

/** Loudness ceiling for every synthesized tone. Kept deliberately subtle. */
const MAX_GAIN = 0.08;
/** Identical cues inside this window collapse into one. */
const THROTTLE_WINDOW_MS = 250;

type AudioContextCtor = new () => AudioContext;

type AudioScope = {
  AudioContext?: AudioContextCtor;
  webkitAudioContext?: AudioContextCtor;
};

type ToneSpec = {
  type: OscillatorType;
  /** Frequency at note start, in Hz. */
  frequency: number;
  /** Optional glide target; a sine blip slides upward to it. */
  endFrequency?: number;
  /** Offset from `currentTime`, in seconds. */
  start: number;
  /** Note length, in seconds. */
  duration: number;
  /** Peak gain for this note; clamped to MAX_GAIN. */
  gain: number;
};

// 'send' rises a little for a just-sent message; 'complete' resolves on a
// soft two-note interval; 'error' is a short low tone; 'notify' is a gentle
// chime with one quiet harmonic.
const CUES: Record<SoundCue, readonly ToneSpec[]> = {
  send: [{ type: 'sine', frequency: 660, endFrequency: 990, start: 0, duration: 0.1, gain: 0.055 }],
  complete: [
    { type: 'sine', frequency: 523.25, start: 0, duration: 0.16, gain: 0.05 },
    { type: 'sine', frequency: 783.99, start: 0.11, duration: 0.22, gain: 0.05 },
  ],
  error: [{ type: 'triangle', frequency: 196, endFrequency: 146.83, start: 0, duration: 0.18, gain: 0.06 }],
  notify: [
    { type: 'sine', frequency: 1046.5, start: 0, duration: 0.22, gain: 0.045 },
    { type: 'sine', frequency: 1567.98, start: 0.02, duration: 0.18, gain: 0.025 },
  ],
};

let enabled = false;
let context: AudioContext | null = null;
let contextUnavailable = false;
let gestureBound = false;
const lastPlayedAt = new Map<SoundCue, number>();

function resolveContextCtor(): AudioContextCtor | null {
  if (typeof globalThis === 'undefined') return null;
  const scope = globalThis as unknown as AudioScope;
  return scope.AudioContext ?? scope.webkitAudioContext ?? null;
}

function resumeContext(ctx: AudioContext): void {
  try {
    if (ctx.state !== 'suspended') return;
    const pending = ctx.resume();
    if (pending && typeof pending.catch === 'function') pending.catch(() => undefined);
  } catch {
    // A suspended or broken context is harmless: later gestures retry.
  }
}

/** Create the single AudioContext on first use; returns null when unsupported. */
function ensureContext(): AudioContext | null {
  if (context) {
    resumeContext(context);
    return context;
  }
  if (contextUnavailable) return null;
  const Ctor = resolveContextCtor();
  if (!Ctor) {
    contextUnavailable = true;
    return null;
  }
  try {
    context = new Ctor();
    resumeContext(context);
    return context;
  } catch {
    context = null;
    contextUnavailable = true;
    return null;
  }
}

// The current one-shot handler, kept module-level so unbind can remove the
// exact function identity the DOM recorded.
let unlockRef: (() => void) | null = null;

function unbindGestureUnlock(): void {
  if (!gestureBound) return;
  gestureBound = false;
  const unlock = unlockRef;
  unlockRef = null;
  if (!unlock || typeof document === 'undefined') return;
  document.removeEventListener('pointerdown', unlock);
  document.removeEventListener('keydown', unlock);
}

// A real gesture (click or key press) unlocks audio on the first interaction
// after sounds are enabled, before any cue has been requested. The listeners
// are one-shot and removed immediately, so they never accumulate.
function installGestureUnlock(): void {
  if (gestureBound || typeof document === 'undefined') return;
  gestureBound = true;
  const unlock = (): void => {
    unbindGestureUnlock();
    if (enabled) ensureContext();
  };
  unlockRef = unlock;
  document.addEventListener('pointerdown', unlock, { passive: true });
  document.addEventListener('keydown', unlock);
}

/** Play one synthesized cue. No-op when sounds are disabled or unsupported. */
export function playCue(cue: SoundCue): void {
  if (!enabled) return;
  try {
    const specs = CUES[cue];
    if (!specs || specs.length === 0) return;
    const stamp = Date.now();
    const previous = lastPlayedAt.get(cue);
    if (previous !== undefined && stamp - previous < THROTTLE_WINDOW_MS) return;
    lastPlayedAt.set(cue, stamp);

    const ctx = ensureContext();
    if (!ctx) return;
    for (const spec of specs) playTone(ctx, spec);
  } catch {
    // Audio is decorative: any failure must leave the UI untouched.
  }
}

function playTone(ctx: AudioContext, spec: ToneSpec): void {
  const startAt = ctx.currentTime + spec.start;
  const stopAt = startAt + spec.duration;
  const peak = Math.min(spec.gain, MAX_GAIN);
  if (peak <= 0) return;

  const oscillator = ctx.createOscillator();
  const gain = ctx.createGain();
  oscillator.type = spec.type;
  oscillator.frequency.setValueAtTime(spec.frequency, startAt);
  if (spec.endFrequency !== undefined && spec.endFrequency > 0) {
    oscillator.frequency.exponentialRampToValueAtTime(spec.endFrequency, stopAt);
  }
  // Short attack, exponential decay: soft edges, no click.
  gain.gain.setValueAtTime(0, startAt);
  gain.gain.linearRampToValueAtTime(peak, startAt + 0.012);
  gain.gain.exponentialRampToValueAtTime(0.0001, stopAt);
  oscillator.connect(gain);
  gain.connect(ctx.destination);
  oscillator.start(startAt);
  oscillator.stop(stopAt + 0.02);
}

/** Global mute flag. Disabling also clears throttle state. */
export function setSoundEnabled(value: boolean): void {
  enabled = Boolean(value);
  if (!enabled) {
    lastPlayedAt.clear();
    return;
  }
  installGestureUnlock();
}

export function isSoundEnabled(): boolean {
  return enabled;
}