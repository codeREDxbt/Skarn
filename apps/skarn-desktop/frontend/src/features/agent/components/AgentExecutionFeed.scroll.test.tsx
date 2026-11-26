import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { agentStore } from '../agent-store';
import type { ChatMessage } from '../types';
import { AgentExecutionFeed } from './AgentExecutionFeed';

const message: ChatMessage = { id: 'scroll-message', role: 'assistant', content: 'Answer', status: 'complete', timestamp: 0 };
const props = { messages: [message], inFlight: false, thinkingState: 'breathing' as const,
  metrics: agentStore.getSnapshot().metrics, promptQueue: [] };

describe('conversation scroll ownership', () => {
  it('preserves each session position without scrolling ancestors', () => {
    const scroll = vi.spyOn(HTMLElement.prototype, 'scrollIntoView');
    const view = render(<AgentExecutionFeed {...props} scrollKey="scroll-first" />);
    const list = view.container.querySelector('.feed-messages-list') as HTMLDivElement;
    Object.defineProperties(list, { scrollHeight: { configurable: true, value: 1000 }, clientHeight: { configurable: true, value: 300 } });
    list.scrollTop = 200;
    fireEvent.scroll(list);
    view.rerender(<AgentExecutionFeed {...props} messages={[{ ...message, content: 'New output' }]} scrollKey="scroll-first" />);
    expect(list.scrollTop).toBe(200);
    view.rerender(<AgentExecutionFeed {...props} scrollKey="scroll-second" />);
    expect(list.scrollTop).toBe(1000);
    list.scrollTop = 500;
    fireEvent.scroll(list);
    view.rerender(<AgentExecutionFeed {...props} scrollKey="scroll-first" />);
    expect(list.scrollTop).toBe(200);
    expect(scroll).not.toHaveBeenCalled();
    scroll.mockRestore();
  });

  it('follows output only when already near the bottom', () => {
    const view = render(<AgentExecutionFeed {...props} scrollKey="scroll-follow" />);
    const list = view.container.querySelector('.feed-messages-list') as HTMLDivElement;
    Object.defineProperties(list, { scrollHeight: { configurable: true, value: 1000 }, clientHeight: { configurable: true, value: 300 } });
    list.scrollTop = 690;
    fireEvent.scroll(list);
    view.rerender(<AgentExecutionFeed {...props} messages={[{ ...message, content: 'More output' }]} scrollKey="scroll-follow" />);
    expect(list.scrollTop).toBe(1000);
  });

  it('extracts multiple think and thinking blocks cleanly without leaking raw tags', () => {
    const view = render(
      <AgentExecutionFeed
        {...props}
        messages={[{
          id: 'multi-think',
          role: 'assistant',
          content: '<think>step one</think>Partial <thinking>step two</thinking>\n\nFinal answer',
          status: 'complete',
          timestamp: 0,
        }]}
        scrollKey="scroll-think"
      />,
    );
    const text = view.container.textContent ?? '';
    expect(text).toContain('Final answer');
    expect(text).toContain('Partial');
    expect(text).not.toContain('<think>');
    expect(text).not.toContain('</think>');
    expect(text).not.toContain('<thinking>');
    expect(text).not.toContain('</thinking>');
  });
});
