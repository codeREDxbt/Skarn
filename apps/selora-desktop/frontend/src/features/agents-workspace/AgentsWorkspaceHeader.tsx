import type { BootstrapState } from '../../bridge/bridge';
import { WindowControls } from '../../layout/TitleBar';
import { SeloraMark } from '../../components/SeloraMark';
import {
  ArrowLeft2,
  ArrowRight2,
  Code2,
  FolderOpen,
  GitBranch,
  Search,
  SidebarLeft,
  SidebarRight,
} from '../../components/icons';

export type AgentsWorkspaceHeaderProps = {
  bootstrap: BootstrapState;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  historyBusy?: boolean;
  navigationOpen: boolean;
  contextOpen: boolean;
  onBack: () => void;
  onForward: () => void;
  onToggleNavigation: () => void;
  onToggleContext: () => void;
  onOpenPalette: () => void;
  onOpenFolder: () => void;
  onReturnEditor: () => void;
};

export function AgentsWorkspaceHeader(props: AgentsWorkspaceHeaderProps) {
  const workspace = props.bootstrap.workspace;
  return <header className="agents-workspace__header" aria-label="Agents workspace" data-tauri-drag-region="">
    <div className="agents-workspace__identity" data-tauri-drag-region=""><SeloraMark size={18} /><span>Agents</span></div>
    <button type="button" onClick={props.onToggleNavigation} aria-expanded={props.navigationOpen} aria-controls="agents-workspace-navigation" aria-label="Toggle Agents navigation" title="Toggle navigation (Ctrl+B)">
      <SidebarLeft size={15} aria-hidden="true" />
    </button>
    <nav className="agents-workspace__history" aria-label="Visit history">
      <button type="button" disabled={!props.canGoBack || props.historyBusy} onClick={props.onBack} aria-label="Go back" title="Go back">
        <ArrowLeft2 size={13} aria-hidden="true" />
      </button>
      <button type="button" disabled={!props.canGoForward || props.historyBusy} onClick={props.onForward} aria-label="Go forward" title="Go forward">
        <ArrowRight2 size={13} aria-hidden="true" />
      </button>
    </nav>
    <span className="agents-workspace__header-title" title={props.title} data-tauri-drag-region="">{props.title}</span>
    <button type="button" className="agents-workspace__workspace" onClick={props.onOpenFolder} title={workspace.root ?? 'Open folder'} aria-label={`Open folder. Current workspace: ${workspace.display_name}`}>
      <FolderOpen size={13} aria-hidden="true" />
      <span>{workspace.display_name}</span>
    </button>
    {props.bootstrap.git.branch && <span className="agents-workspace__branch" title={`Branch: ${props.bootstrap.git.branch}`}><GitBranch size={13} aria-hidden="true" />{props.bootstrap.git.branch}</span>}
    <button type="button" onClick={props.onOpenPalette} title="Command Palette (Ctrl+Shift+P)" aria-label="Open Command Palette"><Search size={15} aria-hidden="true" /></button>
    <button type="button" className="agents-workspace__header-context" onClick={props.onToggleContext} aria-expanded={props.contextOpen} aria-controls="agents-workspace-context" title="Toggle context (Ctrl+Alt+B)">
      <SidebarRight size={14} aria-hidden="true" />
      <span>Context</span>
    </button>
    <button type="button" className="agents-workspace__header-editor" onClick={props.onReturnEditor} title="Return to editor">
      <Code2 size={14} aria-hidden="true" />
      <span>Editor</span>
    </button>
    <WindowControls />
  </header>;
}
