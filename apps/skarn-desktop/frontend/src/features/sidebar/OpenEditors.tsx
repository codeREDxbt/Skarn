import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from 'react';
import { revealInExplorer } from '../../bridge/bridge';
import { currentEditorContext, editorStore } from '../editor/editor-store';
import { absolutePathFor, copyTextToClipboard, relativePathFor } from '../editor/tab-actions';
import { toastStore } from '../toasts/toast-store';
import { FileTypeIcon } from './FileTreeView';
import './open-editors.css';

type Action = 'reveal' | 'copy-path' | 'copy-relative' | 'save' | 'close' | 'close-others' | 'close-saved' | 'close-all';
const actions: Array<{ action: Action; label: string } | 'separator'> = [
  { action: 'reveal', label: 'Reveal in File Explorer' },
  { action: 'copy-path', label: 'Copy Path' },
  { action: 'copy-relative', label: 'Copy Relative Path' },
  'separator', { action: 'save', label: 'Save' },
  'separator', { action: 'close', label: 'Close' },
  { action: 'close-others', label: 'Close Others' },
  { action: 'close-saved', label: 'Close Saved' },
  { action: 'close-all', label: 'Close All' },
];

export function OpenEditors() {
  const editor = useSyncExternalStore(editorStore.subscribe, editorStore.getSnapshot);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [prompt, setPrompt] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const target = editor.tabs.find((tab) => tab.id === menu?.id);
  const promptTab = editor.tabs.find((tab) => tab.id === prompt);

  useLayoutEffect(() => {
    if (!menu || !menuRef.current) return;
    const rect = menuRef.current.getBoundingClientRect();
    setPosition({ left: Math.max(8, Math.min(menu.x, window.innerWidth - rect.width - 8)), top: Math.max(8, Math.min(menu.y, window.innerHeight - rect.height - 8)) });
    menuRef.current.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus();
  }, [menu]);
  useEffect(() => {
    if (!menu) return;
    const dismiss = (event: MouseEvent) => { if (!menuRef.current?.contains(event.target as Node)) setMenu(null); };
    window.addEventListener('mousedown', dismiss);
    return () => window.removeEventListener('mousedown', dismiss);
  }, [menu]);

  const run = (action: Action) => {
    if (!target) return;
    const current = editorStore.getSnapshot();
    if (action === 'reveal') {
      const context = currentEditorContext();
      if (context) void revealInExplorer(target.path, context).catch(() => undefined);
      else toastStore.pushLocal('warning', 'Connect to a workspace before revealing a file.');
    } else if (action === 'copy-path') void copyTextToClipboard(absolutePathFor(current.workspaceRoot, target.path), 'path');
    else if (action === 'copy-relative') void copyTextToClipboard(relativePathFor(target.path), 'relative path');
    else if (action === 'save') void editorStore.saveBuffer(target.id);
    else if (action === 'close') {
      if (target.isDirty) setPrompt(target.id);
      else editorStore.closeBuffer(target.id);
    } else {
      const tabs = current.tabs.filter((tab) => action === 'close-others' ? tab.id !== target.id : action === 'close-saved' ? !tab.isDirty : true);
      const result = editorStore.closeMany(tabs.map((tab) => tab.id));
      if (result.kept) toastStore.pushLocal('info', `${result.kept} ${result.kept === 1 ? 'tab' : 'tabs'} stayed open (unsaved or kept open).`);
    }
    setMenu(null);
  };
  const menuKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); setMenu(null); return; }
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
    if (!items.length) return;
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[next]?.focus();
  };

  if (!editor.tabs.length) return null;
  return <section className="open-editors" aria-label="Open editors">
    <div className="file-tree-action-bar"><span className="explorer-section-title">Open Editors</span></div>
    {editor.tabs.map((tab) => <button key={tab.id} type="button" className={`tree-node-row open-editor-row${tab.id === editor.activeBufferId ? ' is-selected' : ''}`}
      title={tab.path} onClick={() => editorStore.setActive(tab.id)}
      onContextMenu={(event) => { event.preventDefault(); setMenu({ id: tab.id, x: event.clientX, y: event.clientY }); }}
      onKeyDown={(event) => { if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) { event.preventDefault(); const rect = event.currentTarget.getBoundingClientRect(); setMenu({ id: tab.id, x: rect.left, y: rect.bottom }); } }}>
      <FileTypeIcon name={tab.filename} /><span className="tree-item-name">{tab.filename}</span>
      {tab.isDirty ? <span className="tree-dirty-dot" aria-label="Modified">M</span> : null}
    </button>)}
    {menu && target ? <div ref={menuRef} className="tree-context-menu tab-context-menu" role="menu" aria-label={`Actions for ${target.filename}`} style={position} onKeyDown={menuKeyDown} onContextMenu={(event) => event.preventDefault()}>
      {actions.map((item, index) => item === 'separator' ? <div key={index} role="separator" className="context-menu-divider" /> : <button key={item.action} type="button" role="menuitem" className="context-menu-item" onClick={() => run(item.action)}>{item.label}</button>)}
    </div> : null}
    {promptTab ? <div className="tree-context-menu" role="dialog" aria-label={`Unsaved changes in ${promptTab.filename}`} style={{ top: 40, left: 8 }} onKeyDown={(event) => { if (event.key === 'Escape' && !saving) setPrompt(null); }}>
      <div className="breadcrumb-dropdown-title">Unsaved changes</div>
      <button type="button" className="context-menu-item" disabled={saving} onClick={() => { setSaving(true); void editorStore.saveBuffer(promptTab.id).then((saved) => { if (saved) { editorStore.closeBuffer(promptTab.id); setPrompt(null); } setSaving(false); }); }}>Save</button>
      <button type="button" className="context-menu-item is-danger" disabled={saving} onClick={() => { editorStore.closeBuffer(promptTab.id, true); setPrompt(null); }}>Discard</button>
      <button type="button" className="context-menu-item" disabled={saving} onClick={() => setPrompt(null)}>Cancel</button>
    </div> : null}
  </section>;
}
