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
    /** Discovery-reported display name when present (e.g. "Local / local-model" for profiles). */
    discoveryDisplay?: string
    /** Profile display label when this choice belongs to an LLM profile. */
    profileLabel?: string
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

function inheritUnsetSecondarySettings(
    value: ModelSettings,
    inherited: ModelSettings | undefined,
): Partial<ModelSettings> {
    if (!inherited) return {}
    const out: Partial<ModelSettings> = {}
    if (value.thinking != null) {
        out.thinking = value.thinking
        if (value.thinking === 'budget') {
            if (value.thinking_budget_tokens != null) {
                out.thinking_budget_tokens = value.thinking_budget_tokens
            } else if (inherited.thinking === 'budget' && inherited.thinking_budget_tokens !== undefined) {
                out.thinking_budget_tokens = inherited.thinking_budget_tokens
            }
        }
    } else if (inherited.thinking !== undefined) {
        out.thinking = inherited.thinking
        if (value.thinking_budget_tokens == null && inherited.thinking_budget_tokens !== undefined) {
            out.thinking_budget_tokens = inherited.thinking_budget_tokens
        }
    }
    if (value.reasoning_mode == null && inherited.reasoning_mode !== undefined) {
        out.reasoning_mode = inherited.reasoning_mode
    }
    if (value.reasoning_summary == null && inherited.reasoning_summary !== undefined) {
        out.reasoning_summary = inherited.reasoning_summary
    }
    if (value.session_config === undefined && inherited.session_config !== undefined) {
        out.session_config = { ...inherited.session_config }
    }
    return out
}

export function effectiveModelSettings(value: ModelSettings, inherited?: ModelSettings): ModelSettings {
    const inherits = !value.provider && !value.llm_profile
    return {
        ...value,
        ...(inherits ? inheritUnsetSecondarySettings(value, inherited) : {}),
        provider: value.provider || (!value.llm_profile ? inherited?.provider : null) || null,
        llm_profile: value.llm_profile || (!value.provider ? inherited?.llm_profile : null) || null,
        model: value.model || (inherits ? inherited?.model : null) || null,
        reasoning_effort: value.reasoning_effort || (inherits ? inherited?.reasoning_effort : null) || null,
    }
}

/**
 * Pins the same inherited effective model, preserving any secondary settings explicitly authored on
 * the current scope while inheriting only fields the current scope leaves unset.
 * Actual provider/model switches use `selectModelSettings`.
 */
export function pinInheritedModelSettings(
    value: ModelSettings,
    inherited: ModelSettings | undefined,
    pinTarget: { provider: string | null; llm_profile: string | null; model: string | null },
): ModelSettings {
    return {
        ...value,
        ...inheritUnsetSecondarySettings(value, inherited),
        provider: pinTarget.provider,
        llm_profile: pinTarget.llm_profile,
        model: pinTarget.model || null,
    }
}

export function isChoiceAvailable(
    choice: ModelChoiceRef,
    profiles: LlmProfileMetadata[],
    discovery: ModelDiscoveryState | null,
): boolean {
    if (choice.llm_profile) {
        const profile = profiles.find((entry) => entry.id === choice.llm_profile)
        return Boolean(profile && profile.configured)
    }
    if (choice.provider) {
        return !isProviderUnavailable(discovery, choice.provider) && !isProviderChecking(discovery, choice.provider)
    }
    return false
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
    const displayModel = settings.llm_profile
        ? (model || meta?.id || meta?.display || undefined)
        : (meta?.display || model || meta?.id || undefined)
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
            const meta = group.profile
                ? discovery?.payload?.models.find((entry) => entry.llm_profile === group.profile && entry.id === model)
                : findModelMetadata(discovery, group.provider, model)
            const groupLabel = group.profile ? `${labelOf(group.provider, statuses)} / ${group.label}` : labelOf(group.provider, statuses)
            return {
                groupIndex,
                groupLabel,
                providerKey: group.provider,
                provider: group.profile ? null : group.provider,
                llm_profile: group.profile,
                model,
                label: group.profile ? model : (meta?.display || model),
                discoveryDisplay: meta?.display || undefined,
                profileLabel: group.profile ? group.label : undefined,
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

const MODEL_GENERATION_RE = /^(.+?)[\s-]+(\d{1,2})(?:[.-](\d{1,2}))?(?=$|[\s\-/)])/

function parseGenerationText(text: string): { family: string; version: [number, number] } | null {
    const match = MODEL_GENERATION_RE.exec(text.trim())
    if (!match) return null
    const family = match[1].toLowerCase().replace(/[-_\s]+/g, ' ').trim()
    if (!/[a-z]/.test(family)) return null
    return {
        family,
        version: [Number(match[2]), match[3] ? Number(match[3]) : 0],
    }
}

export function parseModelGeneration(
    choice: Pick<ModelChoiceOption, 'label' | 'model'>,
): { family: string; version: [number, number] } | null {
    return parseGenerationText(choice.label) ?? parseGenerationText(choice.model)
}

function compareVersions(a: [number, number], b: [number, number]): number {
    return a[0] !== b[0] ? a[0] - b[0] : a[1] - b[1]
}

/**
 * Splits a provider group's models into current-generation (`primary`) and strictly older (`older`)
 * entries within each named family. Unversioned models, defaults, and `isAlwaysVisible` entries
 * stay in `primary`.
 */
export function partitionGroupGenerations<T extends Pick<ModelChoiceOption, 'label' | 'model' | 'isDefault'>>(
    entries: T[],
    isAlwaysVisible?: (entry: T) => boolean,
): { primary: T[]; older: T[] } {
    // ponytail: name-based version heuristic; replace if providers start returning generation metadata.
    const parsed = entries.map((entry) => parseModelGeneration(entry))
    const latestByFamily = new Map<string, [number, number]>()
    for (const gen of parsed) {
        if (!gen) continue
        const current = latestByFamily.get(gen.family)
        if (!current || compareVersions(gen.version, current) > 0) {
            latestByFamily.set(gen.family, gen.version)
        }
    }
    const primary: T[] = []
    const older: T[] = []
    entries.forEach((entry, index) => {
        const gen = parsed[index]
        const latest = gen ? latestByFamily.get(gen.family) : undefined
        if (!gen || !latest || entry.isDefault || isAlwaysVisible?.(entry) || compareVersions(gen.version, latest) >= 0) {
            primary.push(entry)
        } else {
            older.push(entry)
        }
    })
    return { primary, older }
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

function primaryCandidateKeys(choice: ModelChoiceOption, statuses?: ProjectChatModelsResponse['providers']): string[] {
    const providerDisplay = labelOf(choice.providerKey, statuses)
    const keys: string[] = [choice.model, choice.label]
    if (choice.discoveryDisplay) keys.push(choice.discoveryDisplay)
    if (choice.llm_profile) {
        keys.push(`${choice.llm_profile}/${choice.model}`, `${choice.llm_profile}/${choice.label}`, `${choice.groupLabel}/${choice.model}`)
        if (choice.profileLabel) {
            keys.push(`${choice.profileLabel}/${choice.model}`, `${choice.profileLabel} / ${choice.model}`)
        }
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

function providerAliasKeys(choice: ModelChoiceOption, statuses?: ProjectChatModelsResponse['providers']): string[] {
    if (!choice.llm_profile) return []
    const providerDisplay = labelOf(choice.providerKey, statuses)
    const keys = [
        `${choice.providerKey}/${choice.model}`,
        `${providerDisplay}/${choice.model}`,
        `${choice.providerKey}/${choice.label}`,
        `${providerDisplay}/${choice.label}`,
    ]
    return [...new Set(keys.map((key) => key.trim().toLowerCase()).filter(Boolean))]
}

function allCandidateKeys(choice: ModelChoiceOption, statuses?: ProjectChatModelsResponse['providers']): string[] {
    return [...new Set([...primaryCandidateKeys(choice, statuses), ...providerAliasKeys(choice, statuses)])]
}

function formatAmbiguityError(trimmed: string, matches: ModelChoiceOption[]): string {
    const selectors = matches.slice(0, 5).map(choiceSelectorLabel)
    const more = matches.length > 5 ? ` (+${matches.length - 5} more)` : ''
    return `Multiple models match "${trimmed}": ${selectors.join(', ')}${more}. Specify one of these selectors (e.g. /model ${selectors[0]}).`
}

/** Matches `/model <query>` against available models by exact id/display/selector first, then profile provider/model, then unique substring. */
export function matchAvailableModel(
    query: string,
    rows: ModelChoiceOption[],
    discovery: ModelDiscoveryState | null,
    profiles: LlmProfileMetadata[] = [],
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
    const availableRows = rows.filter((choice) => isChoiceAvailable(choice, profiles, discovery))
    const unconfiguredRows = rows.filter(
        (choice) => Boolean(choice.llm_profile) && !isChoiceAvailable(choice, profiles, discovery),
    )
    const missingCredentialsError = (choice: ModelChoiceOption) => {
        const profile = profiles.find((entry) => entry.id === choice.llm_profile)
        const name = profile?.label || choice.llm_profile || choice.groupLabel
        return `Profile "${name}" is missing required credentials. Configure it in Settings › Models & accounts before selecting it.`
    }

    const exactPrimary = availableRows.filter((choice) => primaryCandidateKeys(choice, statuses).includes(normalized))
    if (exactPrimary.length === 1) {
        return { ok: true, choice: exactPrimary[0] }
    }
    if (exactPrimary.length > 1) {
        return { ok: false, error: formatAmbiguityError(trimmed, exactPrimary) }
    }

    const exactProfileProvider = availableRows.filter((choice) => providerAliasKeys(choice, statuses).includes(normalized))
    if (exactProfileProvider.length === 1) {
        return { ok: true, choice: exactProfileProvider[0] }
    }
    if (exactProfileProvider.length > 1) {
        return { ok: false, error: formatAmbiguityError(trimmed, exactProfileProvider) }
    }

    const exactUnconfigured = unconfiguredRows.filter((choice) => allCandidateKeys(choice, statuses).includes(normalized))
    if (exactUnconfigured.length > 0) {
        return { ok: false, error: missingCredentialsError(exactUnconfigured[0]) }
    }

    const partial = availableRows.filter((choice) => allCandidateKeys(choice, statuses).some((key) => key.includes(normalized)))
    if (partial.length === 1) {
        return { ok: true, choice: partial[0] }
    }
    if (partial.length > 1) {
        return { ok: false, error: formatAmbiguityError(trimmed, partial) }
    }
    const partialUnconfigured = unconfiguredRows.filter((choice) => allCandidateKeys(choice, statuses).some((key) => key.includes(normalized)))
    if (partialUnconfigured.length > 0) {
        return { ok: false, error: missingCredentialsError(partialUnconfigured[0]) }
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
        ? pinInheritedModelSettings(value, inherited, resolved.pinTarget!)
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
