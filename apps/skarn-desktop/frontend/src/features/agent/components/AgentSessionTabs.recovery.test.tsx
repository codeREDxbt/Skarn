import React from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AgentSessionTabs } from './AgentSessionTabs';
import type { AgentSession } from '../types';

afterEach(cleanup);

const sessions: AgentSession[] = ['first', 'second'].map((id) => ({
  id, title: id, createdAt: 0, updatedAt: 0, tokenCount: 0, costUsd: 0, isRunning: false,
}));

describe('session tab actions', () => {
  it('orders pinned tabs and targets keyboard menu actions to the selected tab', () => {
    const pin = vi.fn();
    const select = vi.fn();
    render(<AgentSessionTabs sessions={sessions} activeSessionId="first" pinnedSessionIds={['second']}
      onNew={vi.fn()} onSelect={select} onTogglePin={pin} onDelete={vi.fn()} />);
    const tabs = screen.getAllByRole('tab');
    expect(tabs[0].textContent).toContain('second');
    fireEvent.keyDown(tabs[0], { key: 'ArrowRight' });
    expect(select).toHaveBeenCalledWith('first');
    fireEvent.keyDown(tabs[0], { key: 'F10', shiftKey: true });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Unpin session' }));
    expect(pin).toHaveBeenCalledWith('second');
  });

  it('requires confirmation before deleting the targeted session', () => {
    const remove = vi.fn();
    render(<AgentSessionTabs sessions={sessions} activeSessionId="first"
      onNew={vi.fn()} onSelect={vi.fn()} onTogglePin={vi.fn()} onDelete={remove} />);
    fireEvent.contextMenu(screen.getByRole('tab', { name: 'second' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete session' }));
    expect(remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    expect(remove).toHaveBeenCalledWith('second');
  });
});
