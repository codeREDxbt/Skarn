// Selora IDE — Phase Z5b About dialog + real update check.
//
// UI_SPEC §11: 400x280 modal, 48px mark, name, version, MIT, repository, and
// a Check for updates action. The version is injected by Vite from
// package.json; the update check reads GitHub's latest-release endpoint and
// compares a numeric triple. Failures are shown inline and in a toast, never
// converted into a fabricated success.
import { useEffect, useRef, useState } from 'react';
import { MetalBrand } from '../features/onboarding/BrandFx';
import { toastStore } from '../features/toasts/toast-store';
import { currentEnvelopeContext } from '../app/app-store';
import { openExternal, openLogs, UPDATES_ENABLED } from '../bridge/bridge';
import '../styles/onboarding.css';

declare const __APP_VERSION__: string;
declare const __REPOSITORY_URL__: string;

export const ABOUT_EVENT = 'skarn:about';

// Derived from package.json repository.url via vite.config.ts; CI's
// scripts/check-release-metadata.ps1 asserts Cargo.toml agrees. A rename must
// not leave the update check on a stale slug (a 404 looked like "no release").
export const REPOSITORY_URL = __REPOSITORY_URL__;

/** `owner/repo` from the repository URL, without scheme or `.git`. */
export function repositorySlug(url: string = REPOSITORY_URL): string {
  return url.replace(/^https?:\/\/github\.com\//, '').replace(/\.git$/, '').replace(/\/+$/, '');
}

export const LATEST_RELEASE_URL = `https://api.github.com/repos/${repositorySlug()}/releases/latest`;

export type ReleaseInfo = {
  tag: string;
  htmlUrl: string;
  newer: boolean;
};

type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export function versionTriple(value: string): [number, number, number] | null {
  const tag = value.trim().replace(/^v/, '');
  const parts = tag.split('.');
  if (parts.length !== 3) return null;
  const numbers = parts.map((part) => (/^\d+$/.test(part) ? Number(part) : Number.NaN));
  if (numbers.some((number) => !Number.isSafeInteger(number))) return null;
  return [numbers[0], numbers[1], numbers[2]];
}

export function isNewerVersion(latest: string, current: string): boolean {
  const next = versionTriple(latest);
  const running = versionTriple(current);
  if (!next || !running) return false;
  for (let index = 0; index < 3; index += 1) {
    if (next[index] !== running[index]) return next[index] > running[index];
  }
  return false;
}

export async function checkForUpdate(
  currentVersion: string = __APP_VERSION__,
  fetcher: FetchLike = fetch,
): Promise<ReleaseInfo> {
  if (!UPDATES_ENABLED) {
    throw new Error('In-app updates are disabled in this build (release channel not configured yet).');
  }
  const response = await fetcher(LATEST_RELEASE_URL, {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) {
    throw new Error(`GitHub Releases answered ${response.status}`);
  }
  const payload: unknown = await response.json();
  if (!payload || typeof payload !== 'object') {
    throw new Error('Unreadable release payload');
  }
  const record = payload as Record<string, unknown>;
  if (typeof record.tag_name !== 'string' || typeof record.html_url !== 'string') {
    throw new Error('Release payload is missing tag_name or html_url');
  }
  return {
    tag: record.tag_name,
    htmlUrl: record.html_url,
    newer: isNewerVersion(record.tag_name, currentVersion),
  };
}

/** Subscribe a dialog opener to the command registry's window event. */
export function useAboutTrigger(onOpen: () => void): void {
  useEffect(() => {
    const handler = () => onOpen();
    window.addEventListener(ABOUT_EVENT, handler);
    return () => window.removeEventListener(ABOUT_EVENT, handler);
  }, [onOpen]);
}

export type AboutDialogProps = {
  open: boolean;
  onClose: () => void;
};

export function AboutDialog({ open, onClose }: AboutDialogProps) {
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setError(null);
    setNote(null);
    closeRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (open) return;
    const restore = restoreFocusRef.current;
    restoreFocusRef.current = null;
    if (!restore || !restore.isConnected) return;
    const active = document.activeElement;
    const dialogHadFocus = active === document.body || (dialogRef.current?.contains(active) ?? false);
    if (dialogHadFocus) restore.focus();
  }, [open]);

  if (!open) return null;

  const runUpdateCheck = async () => {
    if (!UPDATES_ENABLED) {
      setError(null);
      setNote('In-app updates are disabled in this build (release channel not configured yet).');
      return;
    }
    setChecking(true);
    setError(null);
    setNote(null);
    try {
      const release = await checkForUpdate();
      if (release.newer) {
        setNote(`Version ${release.tag} is available.`);
        toastStore.pushLocal('info', `Skarn ${release.tag} is available.`);
      } else {
        setNote(`You are on the latest release (${release.tag}).`);
        toastStore.pushLocal('success', `Skarn is up to date (${release.tag}).`);
      }
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
      toastStore.pushLocal('error', `Update check failed: ${message}`);
    } finally {
      setChecking(false);
    }
  };

  const runOpenLogs = () => {
    const context = currentEnvelopeContext();
    if (!context) {
      toastStore.pushLocal('error', 'Cannot open logs: no workspace context.');
      return;
    }
    void openLogs(context).catch((cause: unknown) => {
      const message = cause instanceof Error ? cause.message : String(cause);
      toastStore.pushLocal('error', `Could not open logs: ${message}`);
    });
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = dialogRef.current?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    );
    if (!focusables || focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dialogRef.current) return;
    const rect = dialogRef.current.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const normX = (x / rect.width - 0.5) * 2;
    const normY = (y / rect.height - 0.5) * 2;
    const tiltX = (normX * 4.5).toFixed(2);
    const tiltY = (normY * -4.5).toFixed(2);
    dialogRef.current.style.setProperty('--tilt-x', `${tiltX}deg`);
    dialogRef.current.style.setProperty('--tilt-y', `${tiltY}deg`);
  };

  const handlePointerLeave = () => {
    if (!dialogRef.current) return;
    dialogRef.current.style.setProperty('--tilt-x', '0deg');
    dialogRef.current.style.setProperty('--tilt-y', '0deg');
  };

  return (
    <div className="about-backdrop" data-testid="about-backdrop" onPointerDown={onClose}>
      <div
        ref={dialogRef}
        className="about-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="about-title"
        onPointerDown={(event) => event.stopPropagation()}
        onKeyDown={handleKeyDown}
        onPointerMove={handlePointerMove}
        onPointerLeave={handlePointerLeave}
      >
        <MetalBrand size="about" />
        <h2 id="about-title" className="about-title">
          Skarn
        </h2>
        <p className="about-version">Lite Edition · v{__APP_VERSION__}</p>
        <span className="about-rule" aria-hidden="true" />
        <p className="about-license">Open source · MIT</p>
        <a
          className="about-repo"
          href={REPOSITORY_URL}
          rel="noreferrer"
          onClick={(event) => {
            // A raw anchor cannot navigate inside the Tauri webview:
            // hand the URL to Rust, which checks the host allowlist
            // before the OS browser opens it.
            event.preventDefault();
            const context = currentEnvelopeContext();
            if (!context) {
              toastStore.pushLocal('error', 'Cannot open the repository: no workspace context.');
              return;
            }
            void openExternal(REPOSITORY_URL, context).catch(() => undefined);
          }}
        >
          github.com/{repositorySlug()}
        </a>

        <div className="about-feedback" aria-live="polite">
          {error ? (
            <p className="about-error" role="alert">
              Update check failed: {error}
            </p>
          ) : note ? (
            <p className="about-note">{note}</p>
          ) : null}
        </div>

        <div className="about-actions">
          <button
            type="button"
            className="about-button about-button-primary"
            onClick={() => void runUpdateCheck()}
            disabled={checking || !UPDATES_ENABLED}
            title={
              UPDATES_ENABLED
                ? undefined
                : 'In-app update checks are disabled until releases are published.'
            }
          >
            {checking ? 'Checking…' : UPDATES_ENABLED ? 'Check for updates' : 'Updates disabled'}
          </button>
          <button type="button" className="about-button" onClick={runOpenLogs}>
            Open logs
          </button>
          <button ref={closeRef} type="button" className="about-button" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
