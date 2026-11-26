// Skarn — Dedicated Settings overlay page (region-strict Settings home).
import { useEffect, useRef } from 'react';
import type { BootstrapState } from '../bridge/bridge';
import { SettingsPanelView } from '../features/sidebar/SettingsPanelView';
import { Icon } from './Icon';
import '../styles/cheatsheet.css';
import '../styles/sidebar.css';

export type SettingsDialogProps = {
  open: boolean;
  bootstrap: BootstrapState;
  onClose: () => void;
  onOpenConnect?: () => void;
};

export function SettingsDialog({ open, bootstrap, onClose, onOpenConnect }: SettingsDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (open) return;
    const restore = restoreFocusRef.current;
    restoreFocusRef.current = null;
    if (!restore || !restore.isConnected) return;
    const current = document.activeElement;
    const dialogHadFocus = current === document.body || (dialogRef.current?.contains(current) ?? false);
    if (dialogHadFocus) restore.focus();
  }, [open]);

  if (!open) return null;

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusables = dialogRef.current?.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
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

  return (
    <div className="cheatsheet-backdrop" data-testid="settings-backdrop" onPointerDown={onClose}>
      <div
        ref={dialogRef}
        className="cheatsheet settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-dialog-title"
        onPointerDown={(event) => event.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <header className="cheatsheet-header">
          <div className="cheatsheet-heading">
            <h2 id="settings-dialog-title" className="cheatsheet-title">
              Settings
            </h2>
            <p className="cheatsheet-subtitle">
              Providers, editor preferences, theme, and keyboard shortcuts.
            </p>
          </div>
          <button
            ref={closeRef}
            type="button"
            className="cheatsheet-close"
            aria-label="Close settings"
            onClick={onClose}
          >
            <Icon name="close" />
          </button>
        </header>

        <div className="cheatsheet-body">
          <SettingsPanelView bootstrap={bootstrap} onOpenConnect={onOpenConnect} />
        </div>
      </div>
    </div>
  );
}
