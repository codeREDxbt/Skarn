// Selora IDE - Assistant-provider visibility helpers.
//
// Settings > AI Gateway stores the provider ids the user turned off in
// `selora.disabled_providers`. These pure helpers are the one place that
// decides which provider a backend model id belongs to and how the disabled
// list changes. They gate selection and visibility only: nothing here
// connects, disconnects, switches a model, or makes a network call.

export type ProviderModel = {
  id: string;
  display_name?: string;
};

/**
 * Best-effort provider id for a backend model id/name. The backend reports
 * model ids, not provider ids, so this mirrors the badges the composer shows.
 * An unknown id returns `null` (the provider cannot be inferred, so it is
 * never hidden).
 */
export function providerIdForModel(modelId: string, displayName = ''): string | null {
  const haystack = `${modelId} ${displayName}`.toLowerCase();
  if (!haystack.trim()) return null;
  if (haystack.includes('claude') || haystack.includes('anthropic')) return 'anthropic';
  if (haystack.includes('deepseek')) return 'deepseek';
  if (
    haystack.includes('gpt') ||
    haystack.includes('openai') ||
    /\bo[134](?:[-_ ]|$)/.test(haystack)
  ) {
    return 'openai';
  }
  if (haystack.includes('glm') || haystack.includes('zhipu')) return 'glm';
  if (haystack.includes('ollama') || haystack.includes('llama') || haystack.includes('qwen')) {
    return 'ollama';
  }
  if (haystack.includes('openrouter')) return 'openrouter';
  if (haystack.includes('selora')) return 'selora';
  return null;
}

/**
 * The provider the live session is using, when it can be inferred from the
 * backend's `active_model_id`. `null` means "unknown", and callers must not
 * show a disabled-provider warning for an unknown provider.
 */
export function activeProviderId(
  activeModelId: string | null | undefined,
  models: readonly ProviderModel[],
): string | null {
  if (!activeModelId) return null;
  const matched = models.find((model) => model.id === activeModelId);
  return providerIdForModel(matched?.id ?? activeModelId, matched?.display_name ?? '');
}

/** True when `providerId` is in the persisted disabled list. */
export function isProviderDisabled(
  disabledProviders: readonly string[],
  providerId: string,
): boolean {
  return disabledProviders.includes(providerId);
}

/**
 * The exact list to persist after a toggle. Enabling removes the id;
 * disabling appends it once. The result keeps the existing order, so a
 * no-op toggle returns an equal list.
 */
export function withProviderEnabled(
  disabledProviders: readonly string[],
  providerId: string,
  enabled: boolean,
): string[] {
  const without = disabledProviders.filter((id) => id !== providerId);
  return enabled ? without : [...without, providerId];
}

/**
 * The preset list minus disabled providers. `keepId` (usually the currently
 * selected/active provider) stays visible even when disabled, so a live
 * session is never silently broken or hidden.
 */
export function visiblePresets<T extends { id: string }>(
  presets: readonly T[],
  disabledProviders: readonly string[],
  keepId?: string | null,
): T[] {
  return presets.filter(
    (preset) => preset.id === keepId || !isProviderDisabled(disabledProviders, preset.id),
  );
}