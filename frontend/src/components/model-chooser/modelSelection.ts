import { useSyncExternalStore } from 'react'
import type { ProjectChatModelMetadataResponse, ProjectChatModelsResponse } from '@/lib/api/projectsApi'
import type { ModelSettings } from '@/lib/api/settingsApi'
import { effortNames, providerLabel as labelOf, type LlmProfileMetadata } from '@/lib/llmSuggestions'

export const EMPTY_MODEL_SETTINGS: ModelSettings = {
    provider: null,
    llm_profile: null,
    model: null,
    reasoning_effort: null,
}

export interface ModelChoiceRef {
    provider: string | null
    llm_profile: string | null
    model: string
}

export interface ModelGroup {
    provider: string
    profile: string | null
    label: string
    models: string[]
}

export interface ModelChoiceOption extends ModelChoiceRef {
    groupIndex: number
    groupLabel: string
    providerKey: string
    label: string
    isDefault?: boolean
}

export interface ModelDiscoveryState {
    projectPath: string | null
    payload?: ProjectChatModelsResponse
    failed?: boolean
}

export interface ResolvedModelInfo {
    provider: string | null
    llm_profile: string | null
    model: string | undefined
    effort: string | null | undefined
    meta: ProjectChatModelMetadataResponse | undefined
    efforts: string[]
    unverified: boolean
    providerDisplay: string | null
    /** Atomic target for pinning an inherited selection, or null when no effective model is known. */
    pinTarget: { provider: string | null; llm_profile: string | null; model: string | null } | null
}

export const effortLabel = (effort: string) => effort === 'xhigh' ? 'XHigh' : effort.charAt(0).toUpperCase() + effort.slice(1)

export const isSameModelRef = (a: ModelChoiceRef, b: ModelChoiceRef) =>
    a.provider === b.provider && a.llm_profile === b.llm_profile && a.model === b.model

export function isProviderUnavailable(discovery: ModelDiscoveryState | null, provider: string): boolean {
    return !!discovery && (Boolean(discovery.failed) || discovery.payload?.providers[provider]?.status === 'unavailable')
}

export function isProviderChecking(discovery: ModelDiscoveryState | null, provider: string): boolean {
    return discovery?.payload?.providers[provider]?.status === 'checking'
}

export function findModelMetadata(
    discovery: ModelDiscoveryState | null,
    provider: string | null,
    model: string | null,
): ProjectChatModelMetadataResponse | undefined {
    if (!provider || isProviderUnavailable(discovery, provider)) return undefined
    return discovery?.payload?.models.find(
        (entry) => entry.provider === provider && !entry.llm_profile && (model ? entry.id === model : entry.is_default),
    )
}

export function effectiveModelSettings(value: ModelSettings, inherited?: ModelSettings): ModelSettings {
    const inherits = !value.provider && !value.llm_profile
    return {
        ...value,
        provider: value.provider || (!value.llm_profile ? inherited?.provider : null) || null,
        llm_profile: value.llm_profile || (!value.provider ? inherited?.llm_profile : null) || null,
        model: value.model || (inherits ? inherited?.model : null) || null,
        reasoning_effort: value.reasoning_effort || (inherits ? inherited?.reasoning_effort : null) || null,
    }
}

export function resolveModelSettings(
    settings: ModelSettings,
    profiles: LlmProfileMetadata[],
    discovery: ModelDiscoveryState | null,
): ResolvedModelInfo {
    const statuses = discovery?.payload?.providers
    const profile = settings.llm_profile ? profiles.find((entry) => entry.id === settings.llm_profile) : undefined
    const provider = profile?.provider || settings.provider || null
    const model = settings.model || profile?.default_model || null
    const meta = settings.llm_profile
        ? (profile ? discovery?.payload?.models.find((entry) => entry.llm_profile === profile.id && entry.id === model) : undefined)
        : findModelMetadata(discovery, provider, model)
    const fallback = !settings.llm_profile && provider && !!model && !meta
        ? discovery?.payload?.provider_reasoning_efforts?.[provider]
        : undefined
    const efforts = settings.llm_profile
        ? profile?.reasoning_efforts ?? []
        : meta?.supported_reasoning_efforts ?? fallback ?? []
    const displayModel = meta?.display || model || meta?.id || undefined
    const providerDisplay = settings.llm_profile
        ? (profile?.label || settings.llm_profile)
        : provider
            ? labelOf(provider, statuses)
            : null

    let pinTarget: ResolvedModelInfo['pinTarget'] = null
    if (settings.llm_profile) {
        if (model) {
            pinTarget = { provider: null, llm_profile: settings.llm_profile, model }
        }
    } else if (provider) {
        if (settings.model) {
            pinTarget = { provider, llm_profile: null, model: settings.model }
        } else if (meta) {
            // ACP harness defaults use id: '' in discovery, which stores as model: null.
            pinTarget = { provider, llm_profile: null, model: meta.id || null }
        }
    }

    return {
        provider,
        llm_profile: settings.llm_profile,
        model: displayModel,
        effort: settings.reasoning_effort || meta?.default_reasoning_effort,
        meta,
        efforts,
        unverified: Boolean(meta?.reasoning_unverified || fallback?.length),
        providerDisplay,
        pinTarget,
    }
}

export function buildModelCatalog(
    profiles: LlmProfileMetadata[],
    discovery: ModelDiscoveryState | null,
    excludeAcp = false,
): { groups: ModelGroup[]; rows: ModelChoiceOption[]; unavailableProviders: string[] } {
    const statuses = discovery?.payload?.providers
    const providers = [...new Set([
        ...Object.keys(statuses ?? {}),
        ...(discovery?.payload?.models.map((model) => model.provider) ?? []),
        ...profiles.map((profile) => profile.provider),
    ])].filter((provider) => !excludeAcp || !provider.startsWith('acp:'))

    const groups: ModelGroup[] = providers.flatMap((provider) => {
        const discovered = isProviderUnavailable(discovery, provider)
            ? []
            : discovery?.payload?.models.filter((model) => model.provider === provider && !model.llm_profile) ?? []
        return [
            ...(statuses?.[provider] || discovered.length
                ? [{ provider, profile: null as string | null, label: labelOf(provider, statuses), models: discovered.map((model) => model.id) }]
                : []),
            ...profiles
                .filter((profile) => profile.provider === provider)
                .map((profile) => ({ provider, profile: profile.id, label: profile.label || profile.id, models: profile.models })),
        ]
    })

    const rows: ModelChoiceOption[] = groups.flatMap((group, groupIndex) =>
        [...new Set(group.models)].map((model) => {
            const meta = group.profile ? undefined : findModelMetadata(discovery, group.provider, model)
            const groupLabel = group.profile ? `${labelOf(group.provider, statuses)} / ${group.label}` : labelOf(group.provider, statuses)
            return {
                groupIndex,
                groupLabel,
                providerKey: group.provider,
                provider: group.profile ? null : group.provider,
                llm_profile: group.profile,
                model,
                label: meta?.display || model,
                isDefault: group.profile
                    ? profiles.find((profile) => profile.id === group.profile)?.default_model === model
                    : meta?.is_default,
            }
        }),
    )

    const unavailableProviders = providers.filter(
        (provider) => Boolean(statuses?.[provider]) && isProviderUnavailable(discovery, provider),
    )

    return { groups, rows, unavailableProviders }
}

/**
 * Applies a model selection while preserving cross-provider clearing and capability gating rules.
 * Session controls belong to a harness so switching provider clears them; switching model keeps only
 * native controls offered by the new model (or all of them while the same harness is still checking).
 */
export function selectModelSettings(
    value: ModelSettings,
    selection: { provider: string | null; llm_profile: string | null; model: string | null },
    profiles: LlmProfileMetadata[],
    discovery: ModelDiscoveryState | null,
): ModelSettings {
    const same = selection.provider === value.provider
        && selection.llm_profile === value.llm_profile
        && (selection.model || null) === value.model
    const unknown = !!selection.provider && selection.provider === value.provider && isProviderChecking(discovery, selection.provider)
    const meta = resolveModelSettings({ ...EMPTY_MODEL_SETTINGS, ...selection }, profiles, discovery).meta
    const keep = (current: string | null | undefined, offered?: string[]) =>
        same || unknown || current == null || offered?.includes(current) ? current : null
    const thinking = keep(value.thinking, meta?.supported_thinking)
    return {
        ...value,
        provider: selection.provider,
        llm_profile: selection.llm_profile,
        model: selection.model || null,
        thinking,
        thinking_budget_tokens: thinking === value.thinking ? value.thinking_budget_tokens : null,
        reasoning_mode: keep(value.reasoning_mode, meta?.supported_reasoning_modes),
        reasoning_summary: keep(value.reasoning_summary, meta?.supported_reasoning_summaries),
        session_config: selection.provider === value.provider ? value.session_config : undefined,
    }
}

export type ModelMatchResult =
    | { ok: true; choice: ModelChoiceOption }
    | { ok: false; error: string }

function choiceSelectorLabel(choice: ModelChoiceOption): string {
    const prefix = choice.llm_profile ?? choice.provider ?? choice.providerKey
    return choice.model ? `${prefix}/${choice.model}` : `${prefix} (${choice.label})`
}

function candidateKeys(choice: ModelChoiceOption, statuses?: ProjectChatModelsResponse['providers']): string[] {
    const providerDisplay = labelOf(choice.providerKey, statuses)
    const keys: string[] = [choice.model, choice.label]
    if (choice.llm_profile) {
        keys.push(`${choice.llm_profile}/${choice.model}`, `${choice.llm_profile}/${choice.label}`, `${choice.groupLabel}/${choice.model}`)
    } else if (choice.provider) {
        keys.push(
            `${choice.provider}/${choice.model}`,
            `${choice.provider}/${choice.label}`,
            `${providerDisplay}/${choice.model}`,
            `${providerDisplay}/${choice.label}`,
        )
    }
    return [...new Set(keys.map((key) => key.trim().toLowerCase()).filter(Boolean))]
}

/** Matches `/model <query>` against available models by exact id/display/provider-model first, then unique substring. */
export function matchAvailableModel(
    query: string,
    rows: ModelChoiceOption[],
    discovery: ModelDiscoveryState | null,
): ModelMatchResult {
    const trimmed = query.trim()
    const normalized = trimmed.toLowerCase()
    if (!normalized) {
        return { ok: false, error: 'Specify a model name or id, or run /model to open the picker.' }
    }
    if (!discovery && rows.length === 0) {
        return { ok: false, error: 'Models are still loading. Try again in a moment.' }
    }
    const statuses = discovery?.payload?.providers
    const exact = rows.filter((choice) => candidateKeys(choice, statuses).includes(normalized))
    if (exact.length === 1) {
        return { ok: true, choice: exact[0] }
    }
    if (exact.length > 1) {
        const list = exact.slice(0, 5).map(choiceSelectorLabel).join(', ')
        const more = exact.length > 5 ? ` (+${exact.length - 5} more)` : ''
        return { ok: false, error: `Multiple models match "${trimmed}": ${list}${more}. Specify provider/model.` }
    }
    const partial = rows.filter((choice) => candidateKeys(choice, statuses).some((key) => key.includes(normalized)))
    if (partial.length === 1) {
        return { ok: true, choice: partial[0] }
    }
    if (partial.length > 1) {
        const list = partial.slice(0, 5).map(choiceSelectorLabel).join(', ')
        const more = partial.length > 5 ? ` (+${partial.length - 5} more)` : ''
        return { ok: false, error: `Multiple models match "${trimmed}": ${list}${more}. Be more specific.` }
    }
    return { ok: false, error: `No available model matches "${trimmed}".` }
}

export type EffortCommandResult =
    | { ok: true; next: ModelSettings }
    | { ok: false; error: string }

/** Validates `/effort <level|default>` against the effective model and auto-pins inherited models when needed. */
export function resolveEffortCommand(
    rawLevel: string,
    value: ModelSettings,
    inherited: ModelSettings | undefined,
    profiles: LlmProfileMetadata[],
    discovery: ModelDiscoveryState | null,
): EffortCommandResult {
    const trimmed = rawLevel.trim()
    const effective = effectiveModelSettings(value, inherited)
    const resolved = resolveModelSettings(effective, profiles, discovery)
    const isInherited = !value.provider && !value.llm_profile
    const supported = resolved.efforts
    const status = resolved.provider ? discovery?.payload?.providers?.[resolved.provider] : undefined
    const names = effortNames(status)

    if (!trimmed) {
        return {
            ok: false,
            error: supported.length > 0
                ? `Specify an effort level: ${supported.join(', ')}, or default.`
                : 'Specify an effort level (for example, /effort high or /effort default).',
        }
    }

    if (isInherited && !resolved.pinTarget) {
        return {
            ok: false,
            error: 'No active model is resolved yet. Choose a model with /model before setting effort.',
        }
    }

    const normalized = trimmed.toLowerCase()
    const base = isInherited
        ? selectModelSettings(value, resolved.pinTarget!, profiles, discovery)
        : value

    if (normalized === 'default') {
        return { ok: true, next: { ...base, reasoning_effort: null } }
    }

    const matched = supported.find(
        (entry) => entry.toLowerCase() === normalized || names[entry]?.toLowerCase() === normalized,
    )
    if (!matched) {
        const modelName = resolved.model ?? 'the current model'
        return {
            ok: false,
            error: supported.length > 0
                ? `Effort "${trimmed}" is not supported for ${modelName}. Supported levels: ${supported.join(', ')}, or default.`
                : `${modelName} does not support reasoning effort levels.`,
        }
    }

    return { ok: true, next: { ...base, reasoning_effort: matched } }
}

// Local-only persistence for pinned favorites and recently chosen models.
const RECENTS_STORAGE_KEY = 'spark.model_chooser.recents'
const PINNED_STORAGE_KEY = 'spark.model_chooser.pinned'
const MAX_RECENTS = 5

const preferenceListeners = new Set<() => void>()
let preferenceVersion = 0

function notifyPreferenceListeners() {
    preferenceVersion += 1
    preferenceListeners.forEach((listener) => listener())
}

function readStoredRefs(key: string): ModelChoiceRef[] {
    try {
        const raw = globalThis.localStorage?.getItem(key)
        if (!raw) return []
        const parsed: unknown = JSON.parse(raw)
        if (!Array.isArray(parsed)) return []
        const result: ModelChoiceRef[] = []
        for (const item of parsed) {
            if (!item || typeof item !== 'object' || Array.isArray(item)) continue
            const entry = item as Record<string, unknown>
            const provider = typeof entry.provider === 'string' ? entry.provider : null
            const llm_profile = typeof entry.llm_profile === 'string' ? entry.llm_profile : null
            const model = typeof entry.model === 'string' ? entry.model : null
            if (model === null || (!provider && !llm_profile)) continue
            const ref: ModelChoiceRef = { provider, llm_profile, model }
            if (!result.some((existing) => isSameModelRef(existing, ref))) result.push(ref)
        }
        return result
    } catch {
        return []
    }
}

function writeStoredRefs(key: string, refs: ModelChoiceRef[]) {
    try {
        globalThis.localStorage?.setItem(key, JSON.stringify(refs))
    } catch {
        // Storage unavailable or full; ignore without blocking selection.
    }
    notifyPreferenceListeners()
}

export function getRecentModels(): ModelChoiceRef[] {
    return readStoredRefs(RECENTS_STORAGE_KEY).slice(0, MAX_RECENTS)
}

export function getPinnedModels(): ModelChoiceRef[] {
    return readStoredRefs(PINNED_STORAGE_KEY)
}

export function recordRecentModel(ref: ModelChoiceRef): void {
    if (!ref.provider && !ref.llm_profile) return
    const current = getRecentModels().filter((entry) => !isSameModelRef(entry, ref))
    writeStoredRefs(RECENTS_STORAGE_KEY, [ref, ...current].slice(0, MAX_RECENTS))
}

export function togglePinnedModel(ref: ModelChoiceRef): void {
    if (!ref.provider && !ref.llm_profile) return
    const current = getPinnedModels()
    const exists = current.some((entry) => isSameModelRef(entry, ref))
    const next = exists ? current.filter((entry) => !isSameModelRef(entry, ref)) : [ref, ...current]
    writeStoredRefs(PINNED_STORAGE_KEY, next)
}

function subscribePreferences(listener: () => void): () => void {
    preferenceListeners.add(listener)
    const onStorage = (event: StorageEvent) => {
        if (event.key === RECENTS_STORAGE_KEY || event.key === PINNED_STORAGE_KEY || event.key === null) {
            preferenceVersion += 1
            listener()
        }
    }
    if (typeof window !== 'undefined') window.addEventListener('storage', onStorage)
    return () => {
        preferenceListeners.delete(listener)
        if (typeof window !== 'undefined') window.removeEventListener('storage', onStorage)
    }
}

export function useModelPreferences(): { pinned: ModelChoiceRef[]; recent: ModelChoiceRef[] } {
    useSyncExternalStore(subscribePreferences, () => preferenceVersion, () => 0)
    return {
        pinned: getPinnedModels(),
        recent: getRecentModels(),
    }
}
