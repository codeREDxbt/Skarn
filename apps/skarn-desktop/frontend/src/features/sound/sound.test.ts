// Audio engine tests: no context before opt-in, one context per module,
// 250 ms throttling, unsupported/suspended Web Audio safety, and the gain
// ceiling. AudioContext is fully mocked — no device is touched.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SOUND_PREFERENCE_KEY } from './sound-store';
import type { SoundCue } from './sound';

class MockAudioParam {
  values: number[] = [];
  setValueAtTime(value: number): this {
    this.values.push(value);
    return this;
  }
  linearRampToValueAtTime(value: number): this {
    this.values.push(value);
    return this;
  }
  exponentialRampToValueAtTime(value: number): this {
    this.values.push(value);
    return this;
  }
}

class MockGainNode {
  gain = new MockAudioParam();
  connect(_node: unknown): void {
    // no-op
  }
  disconnect(): void {
    // no-op
  }
}

class MockOscillatorNode {
  type: OscillatorType = 'sine';
  frequency = new MockAudioParam();
  connect(_node: unknown): void {
    // no-op
  }
  start(_when = 0): void {
    // no-op
  }
  stop(_when = 0): void {
    // no-op
  }
}

class MockAudioContext {
  static instances: MockAudioContext[] = [];
  static initialState: AudioContextState = 'running';

  static reset(): void {
    MockAudioContext.instances = [];
    MockAudioContext.initialState = 'running';
  }

  state: AudioContextState = MockAudioContext.initialState;
  currentTime = 0;
  destination = {};
  oscillators: MockOscillatorNode[] = [];
  gains: MockGainNode[] = [];
  resumeCalls = 0;

  constructor() {
    MockAudioContext.instances.push(this);
  }

  createOscillator(): MockOscillatorNode {
    const oscillator = new MockOscillatorNode();
    this.oscillators.push(oscillator);
    return oscillator;
  }

  createGain(): MockGainNode {
    const gain = new MockGainNode();
    this.gains.push(gain);
    return gain;
  }

  resume(): Promise<void> {
    this.resumeCalls += 1;
    this.state = 'running';
    return Promise.resolve();
  }

  suspend(): Promise<void> {
    this.state = 'suspended';
    return Promise.resolve();
  }

  close(): Promise<void> {
    this.state = 'closed';
    return Promise.resolve();
  }
}

function stubAudioContext(): void {
  vi.stubGlobal('AudioContext', MockAudioContext as unknown as typeof AudioContext);
}

async function loadModules() {
  vi.resetModules();
  const sound = await import('./sound');
  const store = await import('./sound-store');
  return { sound, store };
}

const ALL_CUES: readonly SoundCue[] = ['send', 'complete', 'error', 'notify'];

beforeEach(() => {
  localStorage.clear();
  MockAudioContext.reset();
  stubAudioContext();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
});

describe('sound preference', () => {
  it('defaults to OFF and plays nothing without a context', async () => {
    const { sound, store } = await loadModules();

    expect(store.getSnapshot()).toBe(false);
    expect(sound.isSoundEnabled()).toBe(false);
    expect(localStorage.getItem(SOUND_PREFERENCE_KEY)).toBeNull();

    for (const cue of ALL_CUES) store.playCue(cue);

    expect(MockAudioContext.instances).toHaveLength(0);
  });

  it('persists the opt-in under the documented key and notifies subscribers', async () => {
    const { store } = await loadModules();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    store.setEnabled(true);
    expect(store.getSnapshot()).toBe(true);
    expect(localStorage.getItem(SOUND_PREFERENCE_KEY)).toBe('true');
    expect(listener).toHaveBeenCalledTimes(1);

    store.setEnabled(false);
    expect(store.getSnapshot()).toBe(false);
    expect(localStorage.getItem(SOUND_PREFERENCE_KEY)).toBeNull();
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
    store.setEnabled(true);
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it('rehydrates a stored preference', async () => {
    localStorage.setItem(SOUND_PREFERENCE_KEY, 'true');
    const { store } = await loadModules();
    expect(store.getSnapshot()).toBe(true);
  });
});

describe('audio context lifetime', () => {
  it('unlocks the context on the first user gesture after enabling', async () => {
    const { store } = await loadModules();

    // Earlier tests in this file loaded (and enabled) other copies of the
    // module; each installed its own one-shot listener. Drain them first so
    // this test observes only its own module instance.
    document.dispatchEvent(new Event('keydown'));
    MockAudioContext.instances.length = 0;

    store.setEnabled(true);

    // A one-shot listener must fire once, create one context, then detach.
    document.dispatchEvent(new Event('pointerdown'));
    expect(MockAudioContext.instances).toHaveLength(1);

    document.dispatchEvent(new Event('keydown'));
    expect(MockAudioContext.instances).toHaveLength(1);

    store.playCue('send');
    expect(MockAudioContext.instances).toHaveLength(1);
  });

  it('creates exactly one context after opt-in, reused by every cue', async () => {
    const { store } = await loadModules();

    store.setEnabled(true);
    // Enabling alone is lazy: no context until a gesture or the first cue.
    expect(MockAudioContext.instances).toHaveLength(0);

    for (const cue of ALL_CUES) store.playCue(cue);

    expect(MockAudioContext.instances).toHaveLength(1);
    expect(MockAudioContext.instances[0]!.oscillators.length).toBeGreaterThan(0);
  });

  it('throttles identical cues within 250 ms but not different cues', async () => {
    const { store } = await loadModules();
    store.setEnabled(true);

    const clock = vi.spyOn(Date, 'now').mockReturnValue(1_000);
    store.playCue('send');
    const ctx = MockAudioContext.instances[0]!;
    const afterFirst = ctx.oscillators.length;
    expect(afterFirst).toBeGreaterThan(0);

    clock.mockReturnValue(1_100);
    store.playCue('send');
    expect(ctx.oscillators).toHaveLength(afterFirst);

    clock.mockReturnValue(1_150);
    store.playCue('complete');
    expect(ctx.oscillators.length).toBeGreaterThan(afterFirst);

    clock.mockReturnValue(1_260);
    store.playCue('send');
    expect(ctx.oscillators.length).toBeGreaterThan(afterFirst + 2);

    clock.mockRestore();
  });
});

describe('failure safety', () => {
  it('stays silent without throwing when Web Audio is unavailable', async () => {
    vi.stubGlobal('AudioContext', undefined);
    vi.stubGlobal('webkitAudioContext', undefined);
    const { store } = await loadModules();
    store.setEnabled(true);

    expect(() => {
      for (const cue of ALL_CUES) store.playCue(cue);
    }).not.toThrow();
    expect(MockAudioContext.instances).toHaveLength(0);
  });

  it('resumes a suspended context without throwing', async () => {
    MockAudioContext.initialState = 'suspended';
    const { store } = await loadModules();
    store.setEnabled(true);

    expect(() => store.playCue('notify')).not.toThrow();
    const ctx = MockAudioContext.instances[0]!;
    expect(ctx.resumeCalls).toBe(1);
    expect(ctx.oscillators.length).toBeGreaterThan(0);
  });
});

describe('gain ceiling', () => {
  it('keeps every note at or below 0.08', async () => {
    const { store } = await loadModules();
    store.setEnabled(true);
    for (const cue of ALL_CUES) store.playCue(cue);

    const ctx = MockAudioContext.instances[0]!;
    expect(ctx.gains.length).toBeGreaterThan(0);

    const peaks = ctx.gains
      .map((gain) => Math.max(...gain.gain.values))
      .filter((peak) => Number.isFinite(peak));
    expect(peaks.length).toBeGreaterThan(0);
    for (const peak of peaks) {
      expect(peak).toBeGreaterThan(0);
      expect(peak).toBeLessThanOrEqual(0.08);
    }
  });
});