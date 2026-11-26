import { redo, selectAll, undo } from '@codemirror/commands';
import { gotoLine, openSearchPanel } from '@codemirror/search';
import { Transaction } from '@codemirror/state';
import type { EditorView } from '@codemirror/view';
import { toastStore } from '../toasts/toast-store';

export type EditorCommand = 'undo' | 'redo' | 'cut' | 'copy' | 'paste' | 'find' | 'replace' | 'select-all' | 'go-to-line';

/** Use editor transactions so menu edits share keyboard history and buffer sync. */
export async function executeEditorCommand(
  view: EditorView,
  command: EditorCommand,
  isCurrent: () => boolean = () => true,
): Promise<void> {
  view.focus();
  switch (command) {
    case 'go-to-line': gotoLine(view); return;
    case 'undo': undo(view); return;
    case 'redo': redo(view); return;
    case 'select-all': selectAll(view); return;
    case 'find':
    case 'replace':
      openSearchPanel(view);
      view.dom.querySelector<HTMLInputElement>(`input[name="${command === 'replace' ? 'replace' : 'search'}"]`)?.focus();
      return;
  }

  const state = view.state;
  if (state.readOnly && command !== 'copy') return;
  try {
    if (command === 'paste') {
      const text = await navigator.clipboard.readText();
      // Do not paste into a different file or overwrite a newer selection.
      if (!isCurrent() || view.state !== state) return;
      view.dispatch(state.replaceSelection(text), {
        annotations: Transaction.userEvent.of('input.paste'),
        scrollIntoView: true,
      });
    } else {
      const ranges = state.selection.ranges.filter((range) => !range.empty);
      if (ranges.length === 0) return;
      const text = ranges.map((range) => state.sliceDoc(range.from, range.to)).join('\n');
      await navigator.clipboard.writeText(text);
      // A failed clipboard write must never delete the selected text.
      if (command === 'cut' && isCurrent() && view.state === state) {
        view.dispatch({
          changes: ranges.map((range) => ({ from: range.from, to: range.to, insert: '' })),
          annotations: Transaction.userEvent.of('delete.cut'),
          scrollIntoView: true,
        });
      }
    }
  } catch {
    toastStore.pushLocal('error', `Could not ${command} text. Check clipboard permissions.`);
  }
}

/** The title bar and palette dispatch commands without importing CodeMirror. */
export function bindEditorCommands(view: EditorView): () => void {
  let active = true;
  const handler = (event: Event) => {
    const command = (event as CustomEvent<EditorCommand>).detail;
    void executeEditorCommand(view, command, () => active);
  };
  window.addEventListener('skarn:editor-command', handler);
  return () => {
    active = false;
    window.removeEventListener('skarn:editor-command', handler);
  };
}
