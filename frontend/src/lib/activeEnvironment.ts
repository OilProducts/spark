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
// by another environment, or this Spark no longer links it, until it is paired again; or, linked anew since a check
// failed, until Retry checks the new link.
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
// The link the latest switch chose, as it was then: one linked anew in its place is another destination. Whether it is
// still current is `unconfirmed` while a change made in Settings leaves that unknown, until a listing tells.
let destination: EnvironmentLink | undefined
let unconfirmed = false
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

/** Linking an environment again, even with the same origin and label, makes another row: no field changes in place. */
// ponytail: a listing shows a link's fields, not its credential (spark-storage's EnvironmentLink), so a relink the same
// to the second passes as unchanged; list a credential identity if that matters.
const sameLink = (a: EnvironmentLink, b: EnvironmentLink) =>
    a.environment_id === b.environment_id && a.origin === b.origin && a.label === b.label && a.created_at === b.created_at

/** Tells the switcher this machine's links changed; `update` shows the change in its tabs at once, before they reload. */
export function notifyEnvironmentLinksChanged(update?: (links: EnvironmentLink[]) => EnvironmentLink[]) {
    if (update) {
        // A listing already in flight may predate the change; the reload this event starts replaces it.
        refreshSequence += 1
        const previous = state.links
        let links = update(previous)
        // A change Settings confirmed may be older than the row a switch awaits, as an answer that arrives late is: that
        // row stays until the reload shows which is current, unless the change removed that very row.
        const pending = state.selectedEnvironmentId
        if (pending !== null && pending !== state.activeEnvironmentId && destination) {
            const before = previous.find((row) => row.environment_id === pending)
            const after = links.find((row) => row.environment_id === pending)
            const unchanged = before && after ? sameLink(before, after) : before === after
            if (!unchanged && !(before && !after && sameLink(before, destination))) {
                unconfirmed = true
                links = [...links.filter((row) => row.environment_id !== pending), ...(before ? [before] : [])]
            }
        }
        updateState({ links })
        dropRemovedDestination(previous, false)
    }
    window.dispatchEvent(new Event(ENVIRONMENT_LINKS_CHANGED_EVENT))
}

/**
 * A destination removed or linked anew, here or in another tab, while it is checked or its switch awaits approval is no
 * longer one to switch to, whatever its check or the open editors answer: the workspace mounted stays, with its edits.
 * A mounted environment no longer linked fails for good, its live connection closed, until Retry passes or you leave it;
 * one linked anew since a check failed, until Retry checks the new link. This Spark's own mismatch wins over both.
 */
function dropRemovedDestination(previous: EnvironmentLink[], listed = true) {
    const linked = (environmentId: string, links = state.links) => links.find((link) => link.environment_id === environmentId)
    const active = state.activeEnvironmentId
    const now = active === null ? undefined : linked(active)
    const before = active === null ? undefined : linked(active, previous)
    const gone = active !== null && state.linksLoaded && !now
    const replaced = now && before && !sameLink(now, before)
    if (gone || replaced) {
        // Checks still running, sent for the link as it was, find nothing for it: automatic ones, and a Retry's, which
        // runs only while it is the one chosen. A switch elsewhere awaiting approval carries on.
        outdated = reverifySequence
        if (state.selectedEnvironmentId === active) switchSequence += 1
        // Failures they found for the old link no longer apply; this Spark's own mismatch still wins.
        if (gone) applyActiveCheck(unlinked(active))
        else if (now && state.activeCheck.status !== 'ready') applyActiveCheck(relinked(now.label || now.environment_id))
    }
    const pending = state.selectedEnvironmentId
    if (pending === null || pending === active) return
    const current = linked(pending)
    // Short of a listing, only a change that removed the row a switch awaits drops it.
    if (!current || (listed && destination && !sameLink(current, destination))) {
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

const BUNDLE_VERSIONS = `UI API v${EXPECTED_UI_API_VERSION} · protocol v${EXPECTED_PROTOCOL_VERSION}`

/** This Spark, upgraded under the page, no longer speaks its bundle's versions: only a reload helps, whatever a peer answers. */
const describeStalePage = (local: EnvironmentDescriptor) =>
    `This page was loaded for ${BUNDLE_VERSIONS}, but this Spark now runs ${describeEnvironmentVersion(local)}. Reload the page to load its current UI.`

function describeCompatibilityMismatch(name: string, local: EnvironmentDescriptor | null, remote: EnvironmentDescriptor): string {
    if (local && !speaksThisBundle(local)) return describeStalePage(local)
    const here = local ? `this Spark runs ${describeEnvironmentVersion(local)}` : `this page was loaded for ${BUNDLE_VERSIONS}`
    return `${name} runs ${describeEnvironmentVersion(remote)}, while ${here}. Update Spark so both environments match.`
}

const unreachable = (error: string): Check => ({ status: 'unreachable', error, remoteDescriptor: null })
const stalePage = (local: EnvironmentDescriptor, remote: EnvironmentDescriptor | null = null): Check =>
    ({ status: 'incompatible', error: describeStalePage(local), remoteDescriptor: remote })
const unlinked = (name: string): Check => ({ status: 'refused', remoteDescriptor: null, error: `This Spark no longer links ${name}. Link it again in Settings with a new pairing code from ${name}, then Retry.` })
const relinked = (name: string): Check => ({ status: 'refused', remoteDescriptor: null, error: `${name} was linked anew since it was last checked. Retry to check the current link.` })
/** This Spark's own mismatch, once it reports one, wins over whatever a check of a linked environment found. */
const overStalePage = (check: Check): Check => {
    const local = state.localDescriptor
    return local && !speaksThisBundle(local) ? stalePage(local, check.remoteDescriptor) : check
}

// This Spark's descriptor requests, numbered as sent, and the latest one whose answer was recorded.
let localRequests = 0
let localRecorded = 0

/**
 * Asks this Spark for its descriptor and records the answer, unless one to a request sent later came first: an answer
 * sent before an upgrade can arrive after it. One no longer speaking this bundle's versions fails at once the linked
 * environment mounted, its live connection closed, and the one chosen, even awaiting approval, whatever their checks
 * still running find; a check of the one chosen still running only fills in its versions.
 */
function fetchLocalDescriptor(): Promise<EnvironmentDescriptor> {
    const sent = ++localRequests
    const request = fetchEnvironmentDescriptor(null)
    request.then((local) => {
        if (sent < localRecorded) return
        localRecorded = sent
        updateState({ localDescriptor: local })
        if (speaksThisBundle(local)) return
        const { activeEnvironmentId: active, selectedEnvironmentId: selected } = state
        if (active !== null) applyActiveCheck(stalePage(local, state.activeCheck.remoteDescriptor))
        if (selected !== null && selected !== active) {
            if (state.status !== 'checking') switchSequence += 1
            updateState(stalePage(local, state.remoteDescriptor))
        }
    }, () => {})
    return request
}

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
    // This machine's descriptor is asked for alongside. A refused credential, another identity, a link this Spark no
    // longer has or a peer speaking other versions doesn't wait for it, which then explains a mismatch only if already in.
    const localRequest = fetchLocalDescriptor()
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
    // This Spark's proxy answers both with a 404 once it no longer links the environment.
    const missing = status === 404 && found.status === 'rejected' && found.reason instanceof ApiHttpError && found.reason.status === 404
    if (missing) return unlinked(name)
    const remote = found.status === 'fulfilled' ? found.value : null
    const incompatible = (peer: EnvironmentDescriptor, local: EnvironmentDescriptor | null): Check =>
        ({ status: 'incompatible', error: describeCompatibilityMismatch(name, local, peer), remoteDescriptor: peer })
    if (remote && !speaksThisBundle(remote)) return incompatible(remote, localSoFar)
    const [localResult] = await localAnswer
    const local = localResult.status === 'fulfilled' ? localResult.value : null
    if (local && !speaksThisBundle(local)) return stalePage(local, remote)
    if (localResult.status === 'rejected') {
        throw new Error(`Could not verify this Spark server’s version: ${errorText(localResult.reason)}`)
    }
    if (fallback) return unreachable(errorText(error))
    if (!remote) throw error
    return { status: 'ready', error: null, remoteDescriptor: remote }
}

/** Silently loads this machine's descriptor and linked environments for the top-level switcher, each applied as it answers. */
export async function refreshLocalEnvironments(): Promise<void> {
    const seq = ++refreshSequence
    const descriptor = fetchLocalDescriptor().catch(() => {})
    const links = await fetchLocalEnvironmentLinks().catch(() => null)
    if (seq === refreshSequence) {
        // A listing tells whether the row a switch awaits is current; one it can't tell for doesn't go ahead.
        const unknown = unconfirmed && !links
        unconfirmed = false
        if (links) {
            const previous = state.links
            updateState({ links, linksLoaded: true })
            dropRemovedDestination(previous)
        } else if (unknown) {
            switchSequence += 1
            showActiveEnvironment()
        }
    }
    await descriptor
}

/** Resolves once no change made in Settings leaves the row switch `seq` awaits in doubt, or another switch came first. */
function listingSettles(seq: number) {
    return new Promise<void>((resolve) => {
        const settled = () => !unconfirmed || seq !== switchSequence
        if (settled()) return resolve()
        const stop = subscribeActiveEnvironment(() => {
            if (!settled()) return
            stop()
            resolve()
        })
    })
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
    const mount = () => {
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
    }
    const cancel = () => {
        committed = false
        showActiveEnvironment()
    }
    requestNavigation(() => {
        if (committed !== null || seq !== switchSequence) return
        if (!unconfirmed) return mount()
        // Approved while a change made in Settings leaves the destination's row in doubt: once a listing shows that row,
        // open editors are asked again, as edits made meanwhile are theirs.
        committed = false
        void listingSettles(seq).then(() => {
            if (seq === switchSequence) commitEnvironment(seq, environmentId, check, onActivated)
        })
    }, () => {
        if (committed !== null || seq !== switchSequence) return
        cancel()
    }, { live })
    if (committed === null && seq === switchSequence) updateState({ selectedEnvironmentId: environmentId, ...check })
    return committed === true
}

/**
 * Records what a check of the mounted environment found, showing it unless another environment is chosen. One that got
 * no conclusive answer, such as a gateway's while the peer is down, leaves a final failure as it was, so the workspace
 * and its live connection stay closed until a check passes or the user switches environments.
 */
function applyActiveCheck(found: Check) {
    const check = overStalePage(found)
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
    // The link Settings switches to names it in the switcher and remote guidance, though their listing lacks it. One
    // their listing shows otherwise may be newer or older than it: their listing, reloaded alongside its check, tells.
    const link = options?.link
    const listed = state.links.find((entry) => entry.environment_id === environmentId)
    if (link && !listed) updateState({ links: [...state.links, link] })
    unconfirmed = Boolean(link && listed && !sameLink(link, listed))
    if (unconfirmed) void refreshLocalEnvironments()
    destination = link ?? listed
    const active = state.activeEnvironmentId
    if (environmentId === active && (state.selectedEnvironmentId !== active || state.status === 'ready')) {
        showActiveEnvironment()
        if (state.status === 'ready') options?.onActivated?.()
        return state.status === 'ready'
    }
    if (environmentId === null) return commitEnvironment(seq, null, READY_LOCAL, options?.onActivated)

    // Retrying the mounted environment keeps it hidden behind its error until the check passes.
    if (environmentId !== active) updateState({ selectedEnvironmentId: environmentId, status: 'checking', error: null })
    const since = reverifySequence
    const found = await checkRemoteEnvironment(environmentId).catch((error: unknown) => unreachable(errorText(error)))
    await listingSettles(seq)
    if (seq !== switchSequence) return false
    // An automatic check sent after this Retry that concluded first, such as one finding the link gone, decides.
    if (environmentId === active && outdated > since) return false
    const check = overStalePage(found)
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
    // A final error stays until Retry or Return. One found unreachable is checked again, which can find it final while
    // no connection opens.
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
    // A final error shown meanwhile, such as the link found gone, stays though this check, sent before it, passes.
    if (check.status === 'ready' && isFinalCheck(state.activeCheck)) return false
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
    unconfirmed = false
    state = { ...INITIAL_STATE }
    emit()
}
