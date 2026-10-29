// Settings > AI Gateway: the pure helpers behind the Assistant providers card
// and the Connect modal's preset filter. No network, no store.
import { describe, expect, it } from 'vitest';
import {
  activeProviderId,
  isProviderDisabled,
  providerIdForModel,
  visiblePresets,
  withProviderEnabled,
} from './provider-visibility';

const PRESETS = [
  { id: 'selora' },
  { id: 'anthropic' },
  { id: 'openai' },
  { id: 'deepseek' },
  { id: 'ollama' },
];

describe('provider visibility helpers', () => {
  it('maps backend model ids to the Connect preset ids', () => {
    expect(providerIdForModel('claude-sonnet-4')).toBe('anthropic');
    expect(providerIdForModel('anthropic/claude-3.7')).toBe('anthropic');
    expect(providerIdForModel('gpt-4o')).toBe('openai');
    expect(providerIdForModel('o3-mini')).toBe('openai');
    expect(providerIdForModel('deepseek-chat')).toBe('deepseek');
    expect(providerIdForModel('glm-4')).toBe('glm');
    expect(providerIdForModel('llama3.2:latest')).toBe('ollama');
    expect(providerIdForModel('openrouter/auto')).toBe('openrouter');
    expect(providerIdForModel('selora-fast')).toBe('selora');
    expect(providerIdForModel('mystery-model')).toBeNull();
    expect(providerIdForModel('')).toBeNull();
  });

  it('resolves the active provider from the backend catalog without guessing', () => {
    const models = [
      { id: 'deepseek-chat', display_name: 'DeepSeek Chat' },
      { id: 'claude-sonnet-4', display_name: 'Claude Sonnet 4' },
    ];
    expect(activeProviderId('claude-sonnet-4', models)).toBe('anthropic');
    expect(activeProviderId('deepseek-chat', models)).toBe('deepseek');
    expect(activeProviderId('unknown-model', models)).toBeNull();
    expect(activeProviderId(null, models)).toBeNull();
  });

  it('toggles one provider and keeps the persisted order stable', () => {
    expect(withProviderEnabled([], 'deepseek', false)).toEqual(['deepseek']);
    expect(withProviderEnabled(['deepseek'], 'ollama', false)).toEqual(['deepseek', 'ollama']);
    expect(withProviderEnabled(['deepseek', 'ollama'], 'deepseek', true)).toEqual(['ollama']);
    expect(withProviderEnabled(['deepseek'], 'deepseek', false)).toEqual(['deepseek']);
    expect(isProviderDisabled(['deepseek'], 'deepseek')).toBe(true);
    expect(isProviderDisabled(['deepseek'], 'ollama')).toBe(false);
  });

  it('hides disabled presets but keeps the active one visible', () => {
    expect(visiblePresets(PRESETS, ['deepseek', 'ollama']).map((p) => p.id)).toEqual([
      'selora',
      'anthropic',
      'openai',
    ]);
    expect(
      visiblePresets(PRESETS, ['deepseek', 'ollama'], 'deepseek').map((p) => p.id),
    ).toContain('deepseek');
    expect(visiblePresets(PRESETS, [], null)).toHaveLength(PRESETS.length);
  });
});