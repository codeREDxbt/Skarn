import React from 'react';
import { AgentConversationSurface, type AgentConversationSurfaceProps } from '../features/agent/components/AgentConversationSurface';

export type AgentDockProps = Omit<AgentConversationSurfaceProps, 'presentation'>;

/** Default sidebar chrome and the detached Agents window share one conversation. */
export function AgentDock(props: AgentDockProps = {}) {
  return <AgentConversationSurface {...props} />;
}
