import { invoke } from '@tauri-apps/api/core';

export function explorerCreateEntry(workspaceId: string, parentPath: string, name: string, kind: 'file' | 'directory'): Promise<void> {
  return invoke('explorer_create_entry', { workspaceId, parentPath, name, kind });
}

export function explorerRenameEntry(workspaceId: string, path: string, newName: string): Promise<void> {
  return invoke('explorer_rename_entry', { workspaceId, path, newName });
}

export function explorerDeleteEntry(workspaceId: string, path: string): Promise<void> {
  return invoke('explorer_delete_entry', { workspaceId, path });
}
