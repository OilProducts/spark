import { useSyncExternalStore } from 'react'
import {
    ApiHttpError,
    asOptionalString,
    expectObjectRecord,
    expectString,
    fetchJsonWithValidation,
} from './api/shared'

/** The versions this browser bundle speaks: `PROTOCOL_VERSION` and `UI_API_VERSION` in crates/spark-storage/src/environment.rs. */
export const EXPECTED_PROTOCOL_VERSION = 1
export const EXPECTED_UI_API_VERSION = 2

export type EnvironmentDescriptor = {
    environment_id: string
    display_name: string
    protocol_version: number
    ui_api_version: number
    spark_version: string
    /** Only the authenticated descriptor, `GET /workspace/api/environment`, carries it. */
    build_id?: string
}

export type EnvironmentLink = {
    environment_id: string
    origin: string
    label: string
    created_at: string
}

// `refused`: the link no longer reaches the Spark it paired with, which revoked this Spark's credential or was replaced
// by another environment, until it is paired again.
type RemoteEnvironmentStatus = 'ready' | 'checking' | 'incompatible' | 'unreachable' | 'refused'

type ActiveEnvironmentState = {
    /** Environment currently mounted in the workspace (`null` = this local machine). */
    activeEnvironmentId: string | null
    /** Tab currently selected by the user; differs from activeEnvironmentId while checking or blocked. */
    selectedEnvironmentId: string | null
    status: RemoteEnvironmentStatus
    /** What the mounted environment's own last check found: while it failed, its workspace stays hidden, whatever else is checked. */
    activeCheck: Check
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

function parseEnvironmentDescriptor(payload: unknown, endpoint: string): EnvironmentDescriptor {
    const record = expectObjectRecord(payload, endpoint)
    return {
        environment_id: expectString(record.environment_id, endpoint, 'environment_id'),
        display_name: expectString(record.display_name, endpoint, 'display_name'),
        protocol_version: typeof record.protocol_version === 'number' ? record.protocol_version : 0,
        // Missing ui_api_version means an older Spark build that predates the lockstep check.
        ui_api_version: typeof record.ui_api_version === 'number' ? record.ui_api_version : 0,
        spark_version: asOptionalString(record.spark_version) ?? '',
        build_id: asOptionalString(record.build_id),
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

/**
 * An environment's descriptor (`environmentId` `null`: this machine), by default the active one's: the public
 * `/environment`, or the authenticated `/workspace/api/environment`, which also checks the link's credential.
 */
export function fetchEnvironmentDescriptor(
    environmentId: string | null = state.activeEnvironmentId,
    path: '/environment' | '/workspace/api/environment' = '/environment',
): Promise<EnvironmentDescriptor> {
    const url = `${environmentProxyPrefix(environmentId)}${path}`
    return fetchJsonWithValidation(url, undefined, `GET ${url}`, parseEnvironmentDescriptor)
}

export function fetchLocalEnvironmentLinks(): Promise<EnvironmentLink[]> {
    return fetchJsonWithValidation(
        '/workspace/api/environments/links',
        undefined,
        'GET /workspace/api/environments/links',
        parseEnvironmentLinks,
    )
}

type Check = Pick<ActiveEnvironmentState, 'status' | 'error' | 'remoteDescriptor'>
const READY_LOCAL: Check = { status: 'ready', error: null, remoteDescriptor: null }

const INITIAL_STATE: ActiveEnvironmentState = {
    activeEnvironmentId: null,
    selectedEnvironmentId: null,
    status: 'ready',
    activeCheck: READY_LOCAL,
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
// The latest re-check of the mounted environment, and whether one is running. Those numbered up to `outdated` find
// nothing for it: a newer conclusive answer, a passing Retry or a switch came first.
let reverifySequence = 0
let outdated = 0
let reverifying = false

function emit() {
    listeners.forEach((listener) => listener())
}

function updateState(patch: Partial<ActiveEnvironmentState>) {
    state = { ...state, ...patch }
    emit()
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

/** A check that found an environment incompatible, or refusing its link until it is paired again, finds so until a change there. */
export const isFinalCheck = ({ status }: Pick<ActiveEnvironmentState, 'status'>) => status === 'incompatible' || status === 'refused'

/** A failed check, a destination's or the mounted environment's own, hides the workspace behind the error gate. */
export const isWorkspaceBlocked = ({ status, activeCheck }: ActiveEnvironmentState) => (
    (status !== 'ready' && status !== 'checking') || activeCheck.status !== 'ready'
)

/** A leave open editors are asked about: `approved` editors let it pass; once `live` says no, it no longer goes ahead. */
export type NavigationRequest = { proceed: () => void; cancel?: () => void; approved?: object[]; live?: () => boolean }

/**
 * Runs proceed once open editors allow leaving, or cancel when they keep you here; neither runs after a switch to
 * another environment, which the leave would move instead. An editor that holds a leave calls one of them later, or
 * asks again with itself among the `approved` editors, which let it pass, so the others still get their say.
 */
export const requestNavigation = (proceed: () => void, cancel?: () => void, leave: Pick<NavigationRequest, 'approved' | 'live'> = {}) => {
    const environmentId = getActiveEnvironmentId()
    const here = (then?: () => void) => () => { if (getActiveEnvironmentId() === environmentId) then?.() }
    const detail: NavigationRequest = { approved: leave.approved, live: leave.live, proceed: here(proceed), cancel: here(cancel) }
    if (window.dispatchEvent(new CustomEvent('spark:before-navigation', { cancelable: true, detail }))) detail.proceed()
}

export function isRemoteEnvironmentActive(): boolean {
    return state.activeEnvironmentId !== null
}

/** Tells the switcher this machine's links changed; `update` shows the change in its tabs at once, before they reload. */
export function notifyEnvironmentLinksChanged(update?: (links: EnvironmentLink[]) => EnvironmentLink[]) {
    if (update) {
        // A listing already in flight may predate the change; the reload this event starts replaces it.
        refreshSequence += 1
        updateState({ links: update(state.links) })
        dropRemovedDestination()
    }
    window.dispatchEvent(new Event(ENVIRONMENT_LINKS_CHANGED_EVENT))
}

/**
 * A destination removed, here or in another tab, while it is checked or its switch awaits approval is no longer one to
 * switch to, whatever its check or the open editors answer: the workspace mounted stays, with its edits.
 */
function dropRemovedDestination() {
    const pending = state.selectedEnvironmentId
    if (pending !== null && pending !== state.activeEnvironmentId && !state.links.some((link) => link.environment_id === pending)) {
        switchSequence += 1
        showActiveEnvironment()
    }
}

function linkLabelFor(environmentId: string): string {
    const link = state.links.find((entry) => entry.environment_id === environmentId)
    return link?.label || environmentId
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error))

const speaksThisBundle = (descriptor: EnvironmentDescriptor) =>
    descriptor.ui_api_version === EXPECTED_UI_API_VERSION && descriptor.protocol_version === EXPECTED_PROTOCOL_VERSION

export function describeEnvironmentVersion(descriptor: EnvironmentDescriptor): string {
    return [
        descriptor.ui_api_version > 0 ? `UI API v${descriptor.ui_api_version}` : 'unversioned UI API',
        descriptor.protocol_version > 0 ? `protocol v${descriptor.protocol_version}` : 'unversioned protocol',
        descriptor.spark_version ? `Spark ${descriptor.spark_version}` : '',
        descriptor.build_id ? `build ${descriptor.build_id}` : '',
    ].filter(Boolean).join(' · ')
}

function describeCompatibilityMismatch(name: string, local: EnvironmentDescriptor | null, remote: EnvironmentDescriptor): string {
    const expected = `UI API v${EXPECTED_UI_API_VERSION} · protocol v${EXPECTED_PROTOCOL_VERSION}`
    if (local && !speaksThisBundle(local)) {
        return `This page was loaded for ${expected}, but this Spark now runs ${describeEnvironmentVersion(local)}. Reload the page to load its current UI.`
    }
    const here = local ? `this Spark runs ${describeEnvironmentVersion(local)}` : `this page was loaded for ${expected}`
    return `${name} runs ${describeEnvironmentVersion(remote)}, while ${here}. Update Spark so both environments match.`
}

const unreachable = (error: string): Check => ({ status: 'unreachable', error, remoteDescriptor: null })

// The environment that answered a descriptor request: the one its descriptor names, or the other one the proxy refused
// with a 502 naming it in `answered_as`. None answered one the proxy refused naming none, such as a gateway's.
const answeredAs = (result: PromiseSettledResult<EnvironmentDescriptor>): string | undefined => result.status === 'fulfilled'
    ? result.value.environment_id
    : result.reason instanceof ApiHttpError && result.reason.status === 502
        ? asOptionalString((result.reason.payload as { answered_as?: unknown } | null)?.answered_as)
        : undefined

/**
 * Checks a linked environment before its workspace shows: its authenticated descriptor must name it, and it and this
 * machine, as each runs now, must both speak this bundle's versions. A public descriptor, an older peer's or one a
 * gateway's answer leaves to tell, only explains a mismatch or another identity. It throws when this machine or the
 * link's proxy gives no answer, such as while the peer restarts.
 */
async function checkRemoteEnvironment(environmentId: string): Promise<Check> {
    const name = linkLabelFor(environmentId)
    // This machine's descriptor is asked for alongside. A refused credential, another identity or a peer speaking other
    // versions doesn't wait for it, which then explains a mismatch only if already in.
    const localRequest = fetchEnvironmentDescriptor(null)
    let localSoFar = null as EnvironmentDescriptor | null
    localRequest.then((local) => { localSoFar = local }, () => {})
    const localAnswer = Promise.allSettled([localRequest])
    const [authenticated] = await Promise.allSettled([fetchEnvironmentDescriptor(environmentId, '/workspace/api/environment')])
    const error = authenticated.status === 'rejected' ? authenticated.reason : null
    const status = error instanceof ApiHttpError ? error.status : 0
    // A refused credential, another environment answering at the link's origin, or a peer speaking other versions is
    // final whatever this machine answers.
    if (status === 401) {
        return { status: 'refused', remoteDescriptor: null, error: `${name} refused this Spark’s credential (HTTP 401): its peer token was revoked or is invalid. Remove the link in Settings and pair again with a new pairing code from ${name}.` }
    }
    // An older peer has no authenticated descriptor (404); an answer naming no environment (502) may be a gateway's while
    // the peer is down. The public descriptor tells which environment, if any, is there.
    const fallback = (status === 404 || status === 502) && answeredAs(authenticated) === undefined
    const [found] = fallback ? await Promise.allSettled([fetchEnvironmentDescriptor(environmentId)]) : [authenticated]
    const other = answeredAs(found)
    if (other !== undefined && other !== environmentId) {
        return { status: 'refused', remoteDescriptor: null, error: `${name} answered as environment ${other}, not the linked ${environmentId}. Remove the link in Settings and pair again with the Spark you mean.` }
    }
    const remote = found.status === 'fulfilled' ? found.value : null
    const incompatible = (peer: EnvironmentDescriptor, local: EnvironmentDescriptor | null): Check =>
        ({ status: 'incompatible', error: describeCompatibilityMismatch(name, local, peer), remoteDescriptor: peer })
    if (remote && !speaksThisBundle(remote)) {
        if (localSoFar) updateState({ localDescriptor: localSoFar })
        return incompatible(remote, localSoFar)
    }
    const [localResult] = await localAnswer
    const local = localResult.status === 'fulfilled' ? localResult.value : null
    if (local) updateState({ localDescriptor: local })
    if (remote && local && !speaksThisBundle(local)) return incompatible(remote, local)
    if (localResult.status === 'rejected') {
        throw new Error(`Could not verify this Spark server’s version: ${errorText(localResult.reason)}`)
    }
    if (fallback) return unreachable(errorText(error))
    if (!remote) throw error
    return { status: 'ready', error: null, remoteDescriptor: remote }
}

/** Silently loads this machine's descriptor and linked environments for the top-level switcher. */
export async function refreshLocalEnvironments(): Promise<void> {
    const seq = ++refreshSequence
    const [descriptorResult, linksResult] = await Promise.allSettled([
        fetchEnvironmentDescriptor(null),
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
    if (patch.links) dropRemovedDestination()
}

/** Shows the mounted environment again as its own last check left it, without remounting it. */
function showActiveEnvironment() {
    updateState({ selectedEnvironmentId: state.activeEnvironmentId, ...state.activeCheck })
}

/**
 * Mounts `environmentId` in place of the current workspace once its open editors allow leaving, unless a newer
 * switch came first. While they hold the switch the current workspace stays mounted, though no longer the one chosen,
 * so what waits to show in it waits on; approving later commits once, and cancelling shows it again.
 */
function commitEnvironment(seq: number, environmentId: string | null, check: Check, onActivated?: () => void): boolean {
    let committed: boolean | null = null
    const live = () => committed === null && seq === switchSequence
    requestNavigation(() => {
        if (committed !== null || seq !== switchSequence) return
        committed = true
        // Checks of the environment mounted until now, even this same one, find nothing for this mount.
        outdated = reverifySequence
        reverifying = false
        // Update activeEnvironmentId before wiping stores so any store subscription sees the new environment,
        // then notify React listeners once stores are clean so the remounted subtree starts fresh.
        state = { ...state, activeEnvironmentId: environmentId, selectedEnvironmentId: environmentId, activeCheck: check, ...check }
        resetHandler?.(environmentId)
        emit()
        onActivated?.()
    }, () => {
        if (committed !== null || seq !== switchSequence) return
        committed = false
        showActiveEnvironment()
    }, { live })
    if (committed === null && seq === switchSequence) updateState({ selectedEnvironmentId: environmentId, ...check })
    return committed === true
}

/**
 * Records what a check of the mounted environment found, showing it unless another environment is chosen. One that got
 * no conclusive answer, such as a gateway's while the peer is down, leaves a final failure as it was, so the workspace
 * and its live connection stay closed until a check passes or the user switches environments.
 */
function applyActiveCheck(check: Check) {
    if (check.status === 'unreachable' && isFinalCheck(state.activeCheck)) return
    updateState(state.selectedEnvironmentId === state.activeEnvironmentId ? { activeCheck: check, ...check } : { activeCheck: check })
}

/**
 * Switches the workspace to `environmentId` (`null`: this machine). A linked environment is checked first while the
 * current workspace stays mounted; a failed check hides it, keeping its edits, until the user retries or returns. One
 * that failed its own check stays hidden while another is checked.
 */
export async function selectEnvironment(
    environmentId: string | null,
    options?: { onActivated?: () => void; link?: EnvironmentLink },
): Promise<boolean> {
    const seq = ++switchSequence
    // The link Settings switches to names it in the switcher and remote guidance, though their listing lacks it.
    const link = options?.link
    if (link && !state.links.some((entry) => entry.environment_id === link.environment_id)) updateState({ links: [...state.links, link] })
    const active = state.activeEnvironmentId
    if (environmentId === active && (state.selectedEnvironmentId !== active || state.status === 'ready')) {
        showActiveEnvironment()
        if (state.status === 'ready') options?.onActivated?.()
        return state.status === 'ready'
    }
    if (environmentId === null) return commitEnvironment(seq, null, READY_LOCAL, options?.onActivated)

    // Retrying the mounted environment keeps it hidden behind its error until the check passes.
    if (environmentId !== active) updateState({ selectedEnvironmentId: environmentId, status: 'checking', error: null })
    const check = await checkRemoteEnvironment(environmentId).catch((error: unknown) => unreachable(errorText(error)))
    if (seq !== switchSequence) return false
    if (environmentId === active && check.status !== 'unreachable') {
        // A Retry's conclusive check outdates the mounted environment's automatic ones still running, as one sent with
        // a credential pairing again since replaced. One with no conclusive answer leaves theirs to land, such as a
        // refused credential.
        outdated = reverifySequence
        reverifying = false
    }
    if (environmentId === active || check.status !== 'ready') {
        if (environmentId === active) applyActiveCheck(check)
        else updateState(check)
        if (check.status === 'ready') options?.onActivated?.()
        return check.status === 'ready'
    }
    return commitEnvironment(seq, environmentId, check, options?.onActivated)
}

export function retrySelectedEnvironment(): Promise<boolean> {
    return selectEnvironment(state.selectedEnvironmentId)
}

/**
 * Checks the mounted linked environment again, as when its live connection fails or opens again after failing: one
 * that was upgraded, re-paired as another environment or revoked this Spark's credential hides its workspace behind the
 * gate at once. A switch elsewhere carries on: the gate shows its check, and a return from it shows this error. One
 * an outage's check found unreachable comes back once a later check passes.
 * Resolves whether the mounted environment passed, or `null` with no answer. A failing connection's checks run one at a
 * time; a `fresh` one, for a connection that opened again, starts anew, and what it concludes outdates any still running.
 */
export async function reverifyActiveEnvironment(fresh = false): Promise<boolean | null> {
    const environmentId = state.activeEnvironmentId
    if (environmentId === null) return true
    // A final error stays until Retry or Return, or until a connection that opens again passes a check then. One found
    // unreachable is checked again, which can find it final while no connection opens.
    if (isFinalCheck(state.activeCheck) && !fresh) return false
    if (reverifying && !fresh) return null
    const seq = ++reverifySequence
    reverifying = true
    let check: Check
    try {
        check = await checkRemoteEnvironment(environmentId)
    } catch {
        return null
    } finally {
        if (seq === reverifySequence) reverifying = false
    }
    // A newer check decides, apart from a final failure found here, which stands unless it was outdated: a newer one
    // that gets no conclusive answer leaves it to take effect.
    if (seq <= outdated || state.activeEnvironmentId !== environmentId || (seq !== reverifySequence && !isFinalCheck(check))) return null
    if (check.status !== 'unreachable') outdated = seq
    applyActiveCheck(check)
    return check.status === 'ready'
}

export function subscribeActiveEnvironment(listener: () => void) {
    listeners.add(listener)
    return () => {
        listeners.delete(listener)
    }
}

export function useActiveEnvironment(): ActiveEnvironmentState {
    return useSyncExternalStore(subscribeActiveEnvironment, getActiveEnvironmentState, getActiveEnvironmentState)
}

export function resetActiveEnvironmentForTests() {
    switchSequence += 1
    refreshSequence += 1
    outdated = reverifySequence
    reverifying = false
    state = { ...INITIAL_STATE }
    emit()
}
