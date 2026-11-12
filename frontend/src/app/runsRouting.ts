import type { AppState, ViewMode } from '@/store'

export interface RunsRoute {
    runId: string
    nodeId: string | null
}

const RUNS_HASH_PREFIX = '#/runs/'

export function parseRunsHash(hash: string): RunsRoute | null {
    if (!hash.startsWith(RUNS_HASH_PREFIX)) {
        return null
    }
    const segments = hash
        .slice(RUNS_HASH_PREFIX.length)
        .split('/')
        .map((segment) => decodeURIComponent(segment))
        .filter((segment) => segment.length > 0)
    const runId = segments[0]
    if (!runId) {
        return null
    }
    return {
        runId,
        nodeId: segments[1] ?? null,
    }
}

export function buildRunsHash(runId: string, nodeId?: string | null): string {
    const encodedRunId = encodeURIComponent(runId)
    if (!nodeId) {
        return `${RUNS_HASH_PREFIX}${encodedRunId}`
    }
    return `${RUNS_HASH_PREFIX}${encodedRunId}/${encodeURIComponent(nodeId)}`
}

export function isRunsHash(hash: string): boolean {
    return hash.startsWith(RUNS_HASH_PREFIX)
}

const VIEW_HASH = /^#\/(overview|home|projects|missions|editor|triggers|settings|runs|roles)\/?$/

/**
 * Where the app is, as one browser history entry: the view, plus the run open in Runs or the
 * project page open in Chats. Other selections (a trigger, a mission, a node) change in place.
 * An absent field leaves that selection as it is.
 */
export interface Place {
    view: ViewMode
    runId?: string | null
    nodeId?: string | null
    projectPagePath?: string | null
}

export function placeOf(state: AppState): Place {
    const view = state.viewMode === 'projects' ? 'home' : state.viewMode
    if (view === 'home') return { view, projectPagePath: state.projectPagePath }
    if (view !== 'runs') return { view }
    const runId = state.runsListSession.selectedRunId
    return { view, runId, nodeId: runId ? state.runDetailSessionsByRunId[runId]?.selectedNodeId ?? null : null }
}

/** Places with the same key are one history entry; a node change only rewrites the entry. */
export const placeKey = (place: Place) => `${place.view}\n${place.runId ?? ''}\n${place.projectPagePath ?? ''}`

export const placeHash = (place: Place) => (place.runId ? buildRunsHash(place.runId, place.nodeId) : `#/${place.view}`)

export function parsePlaceHash(hash: string): Place | null {
    const run = parseRunsHash(hash)
    if (run) return { view: 'runs', ...run }
    // A bare view link keeps whichever run or project page was open in that view.
    const view = VIEW_HASH.exec(hash)?.[1] as ViewMode | undefined
    return view ? { view: view === 'projects' ? 'home' : view } : null
}
