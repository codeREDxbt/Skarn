// Local success/failure notifications (the cargo build surface).
//
// `pushLocal` is the path the terminal store uses. A success stays a success
// long enough to be read, and the notification history keeps the same
// severity so the notification centre does not lie after the toast fades.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  NOTIFICATION_HISTORY_KEY,
  toastStore,
  TOAST_CALM_MS,
} from './toast-store';

beforeEach(() => {
  toastStore.clearAll();
  toastStore.clearHistory();
  window.localStorage.clear();
});

afterEach(() => {
  toastStore.clearAll();
  toastStore.clearHistory();
});

describe('toastStore local notifications', () => {
  it('pushes a success toast and records it in history', () => {
    const id = toastStore.pushLocal('success', 'Cargo build finished: Finished `dev` profile');

    const snapshot = toastStore.getSnapshot();
    expect(snapshot.toasts).toHaveLength(1);
    expect(snapshot.toasts[0]).toMatchObject({
      id,
      severity: 'success',
      message: 'Cargo build finished: Finished `dev` profile',
    });
    expect(snapshot.history[0]).toMatchObject({
      severity: 'success',
      message: 'Cargo build finished: Finished `dev` profile',
      read: false,
    });
    expect(snapshot.unreadCount).toBe(1);
    expect(TOAST_CALM_MS).toBe(4000);
  });

  it('keeps a failure as an error toast, never a success', () => {
    const id = toastStore.pushLocal('error', 'Cargo build failed: error: could not compile `app`');

    const toast = toastStore.getSnapshot().toasts[0];
    expect(toast).toMatchObject({
      id,
      severity: 'error',
      message: 'Cargo build failed: error: could not compile `app`',
    });
    expect(toastStore.getSnapshot().history[0]?.severity).toBe('error');
  });

  it('persists history under the notification key', () => {
    toastStore.pushLocal('success', 'Cargo build finished');

    const raw = window.localStorage.getItem(NOTIFICATION_HISTORY_KEY);
    expect(raw).not.toBeNull();
    const entries = JSON.parse(raw ?? '[]') as Array<{ message: string; severity: string }>;
    expect(entries[0]).toMatchObject({ message: 'Cargo build finished', severity: 'success' });
  });

  it('notifies subscribers for each local toast', () => {
    const seen: number[] = [];
    const unsubscribe = toastStore.subscribe(() => {
      seen.push(toastStore.getSnapshot().toasts.length);
    });

    toastStore.pushLocal('success', 'one');
    toastStore.pushLocal('error', 'two');
    unsubscribe();

    expect(seen).toEqual([1, 2]);
  });
});