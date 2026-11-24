import { useSyncExternalStore } from 'react'
import {
    asOptionalString,
    expectObjectRecord,
    expectString,
    fetchJsonWithValidation,
} from './api/shared'

export type EnvironmentDescriptor = {
    environment_id: string
    display_name: string
    protocol_version: number
    ui_api_version: number
    spark_version: string
}

export type EnvironmentLink = {
    environment_id: string
    origin: string
    label: string
    created_at: string
}

export type RemoteEnvironmentStatus = 'ready' | 'checking' | 'incompatible' | 'unreachable'

export type ActiveEnvironmentState = {
    /** Environment currently mounted in the workspace (`null` = this local machine). */
    activeEnvironmentId: string | null
    /** Tab currently selected by the user; differs from activeEnvironmentId while checking or blocked. */
    selectedEnvironmentId: string | null
    status: RemoteEnvironmentStatus
    localDescriptor: EnvironmentDescriptor | null
    remoteDescriptor: EnvironmentDescriptor | null
    error: string | null
    links: EnvironmentLink[]
    linksLoaded: boolean
}

export const ENVIRONMENT_LINKS_CHANGED_EVENT = 'spark:environment-links-changed'

export function environmentProxyPrefix(environmentId: string | null | undefined): string {
    return environmentId ? `/workspace/api/environments/links/${encodeURIComponent(environmentId)}/proxy` : ''
}

export function parseEnvironmentDescriptor(payload: unknown, endpoint: string): EnvironmentDescriptor {
    const record = expectObjectRecord(payload, endpoint)
    return {
        environment_id: expectString(record.environment_id, endpoint, 'environment_id'),
        display_name: expectString(record.display_name, endpoint, 'display_name'),
        protocol_version: typeof record.protocol_version === 'number' ? record.protocol_version : 0,
        // Missing ui_api_version means an older Spark build that predates the lockstep check.
        ui_api_version: typeof record.ui_api_version === 'number' ? record.ui_api_version : 0,
        spark_version: asOptionalString(record.spark_version) ?? '',
    }
}

export function parseEnvironmentLink(payload: unknown, endpoint: string): EnvironmentLink {
    const row = expectObjectRecord(payload, endpoint)
    return {
        environment_id: expectString(row.environment_id, endpoint, 'environment_id'),
        origin: expectString(row.origin, endpoint, 'origin'),
        label: asOptionalString(row.label) ?? '',
        created_at: asOptionalString(row.created_at) ?? '',
    }
}

export function parseEnvironmentLinks(payload: unknown, endpoint: string): EnvironmentLink[] {
    const record = expectObjectRecord(payload, endpoint)
    const items = Array.isArray(record.links) ? record.links : []
    return items.map((item) => parseEnvironmentLink(item, endpoint))
}

export function fetchLocalEnvironmentDescriptor(): Promise<EnvironmentDescriptor> {
    return fetchJsonWithValidation('/environment', undefined, 'GET /environment', parseEnvironmentDescriptor)
}

export function fetchRemoteEnvironmentDescriptor(environmentId: string): Promise<EnvironmentDescriptor> {
    const prefix = environmentProxyPrefix(environmentId)
    return fetchJsonWithValidation(`${prefix}/environment`, undefined, `GET ${prefix}/environment`, parseEnvironmentDescriptor)
}

export function fetchLocalEnvironmentLinks(): Promise<EnvironmentLink[]> {
    return fetchJsonWithValidation(
        '/workspace/api/environments/links',
        undefined,
        'GET /workspace/api/environments/links',
        parseEnvironmentLinks,
    )
}

const INITIAL_STATE: ActiveEnvironmentState = {
    activeEnvironmentId: null,
    selectedEnvironmentId: null,
    status: 'ready',
    localDescriptor: null,
    remoteDescriptor: null,
    error: null,
    links: [],
    linksLoaded: false,
}

let state: ActiveEnvironmentState = { ...INITIAL_STATE }
const listeners = new Set<() => void>()
let switchSequence = 0
let refreshSequence = 0
let resetHandler: ((targetEnvironmentId: string | null) => void) | null = null

function updateState(patch: Partial<ActiveEnvironmentState>) {
    state = { ...state, ...patch }
    listeners.forEach((listener) => listener())
}

export function registerEnvironmentResetHandler(handler: (targetEnvironmentId: string | null) => void) {
    resetHandler = handler
}

export function getActiveEnvironmentState(): ActiveEnvironmentState {
    return state
}

export function getActiveEnvironmentId(): string | null {
    return state.activeEnvironmentId
}

export function isRemoteEnvironmentActive(): boolean {
    return state.activeEnvironmentId !== null
}

export function activeApiPrefix(): string {
    return environmentProxyPrefix(state.activeEnvironmentId)
}

export function notifyEnvironmentLinksChanged() {
    window.dispatchEvent(new Event(ENVIRONMENT_LINKS_CHANGED_EVENT))
}

function linkLabelFor(environmentId: string): string {
    const link = state.links.find((entry) => entry.environment_id === environmentId)
    return link?.label || environmentId
}

function describeCompatibilityMismatch(
    name: string,
    local: EnvironmentDescriptor,
    remote: EnvironmentDescriptor,
): string {
    const remoteSpark = remote.spark_version ? `, Spark ${remote.spark_version}` : ''
    const localSpark = local.spark_version ? `, Spark ${local.spark_version}` : ''
    const remoteUi = remote.ui_api_version > 0 ? `UI API v${remote.ui_api_version}` : 'an older UI API (unversioned)'
    return `${name} runs ${remoteUi} (protocol v${remote.protocol_version}${remoteSpark}), while this Spark expects UI API v${local.ui_api_version} (protocol v${local.protocol_version}${localSpark}). Update Spark so both environments match.`
}

/** Silently loads this machine's descriptor and linked environments for the top-level switcher. */
export async function refreshLocalEnvironments(): Promise<void> {
    const seq = ++refreshSequence
    const [descriptorResult, linksResult] = await Promise.allSettled([
        fetchLocalEnvironmentDescriptor(),
        fetchLocalEnvironmentLinks(),
    ])
    if (seq !== refreshSequence) return
    const patch: Partial<ActiveEnvironmentState> = {}
    if (descriptorResult.status === 'fulfilled') {
        patch.localDescriptor = descriptorResult.value
    }
    if (linksResult.status === 'fulfilled') {
        patch.links = linksResult.value
        patch.linksLoaded = true
    }
    if (Object.keys(patch).length > 0) {
        updateState(patch)
    }
}

export async function selectEnvironment(
    environmentId: string | null,
    options?: { force?: boolean; onActivated?: () => void },
): Promise<boolean> {
    if (
        !options?.force
        && environmentId === state.selectedEnvironmentId
        && environmentId === state.activeEnvironmentId
        && state.status === 'ready'
    ) {
        options?.onActivated?.()
        return true
    }

    const seq = ++switchSequence

    if (environmentId === null) {
        const wasRemote = state.activeEnvironmentId !== null
        state = {
            ...state,
            activeEnvironmentId: null,
            selectedEnvironmentId: null,
            status: 'ready',
            remoteDescriptor: null,
            error: null,
        }
        if (wasRemote) {
            resetHandler?.(null)
        }
        listeners.forEach((listener) => listener())
        options?.onActivated?.()
        return true
    }

    updateState({
        selectedEnvironmentId: environmentId,
        status: 'checking',
        remoteDescriptor: null,
        error: null,
    })

    let local = state.localDescriptor
    if (!local) {
        try {
            local = await fetchLocalEnvironmentDescriptor()
            if (seq !== switchSequence) return false
            updateState({ localDescriptor: local })
        } catch (error) {
            if (seq !== switchSequence) return false
            updateState({
                status: 'unreachable',
                error: error instanceof Error
                    ? `Could not verify this Spark server’s version: ${error.message}`
                    : 'Could not verify this Spark server’s version.',
            })
            return false
        }
    }

    let remote: EnvironmentDescriptor
    try {
        remote = await fetchRemoteEnvironmentDescriptor(environmentId)
    } catch (error) {
        if (seq !== switchSequence) return false
        updateState({
            status: 'unreachable',
            remoteDescriptor: null,
            error: error instanceof Error ? error.message : `Could not reach ${linkLabelFor(environmentId)}.`,
        })
        return false
    }

    if (seq !== switchSequence) return false

    const compatible =
        remote.ui_api_version > 0
        && local.ui_api_version > 0
        && remote.ui_api_version === local.ui_api_version
        && remote.protocol_version === local.protocol_version

    if (!compatible) {
        updateState({
            status: 'incompatible',
            remoteDescriptor: remote,
            error: describeCompatibilityMismatch(linkLabelFor(environmentId), local, remote),
        })
        return false
    }

    // Update activeEnvironmentId before wiping stores so any store subscription sees remote mode,
    // then notify React listeners once stores are clean so the remounted subtree starts fresh.
    state = {
        ...state,
        activeEnvironmentId: environmentId,
        selectedEnvironmentId: environmentId,
        status: 'ready',
        remoteDescriptor: remote,
        error: null,
    }
    resetHandler?.(environmentId)
    listeners.forEach((listener) => listener())
    options?.onActivated?.()
    return true
}

export function retrySelectedEnvironment(): Promise<boolean> {
    return selectEnvironment(state.selectedEnvironmentId, { force: true })
}

function subscribe(listener: () => void) {
    listeners.add(listener)
    return () => {
        listeners.delete(listener)
    }
}

export function useActiveEnvironment(): ActiveEnvironmentState {
    return useSyncExternalStore(subscribe, getActiveEnvironmentState, getActiveEnvironmentState)
}

export function resetActiveEnvironmentForTests() {
    switchSequence += 1
    refreshSequence += 1
    state = { ...INITIAL_STATE }
    listeners.forEach((listener) => listener())
}
