// eslint-disable-next-line @typescript-eslint/no-restricted-imports -- triggerKey names a trigger; it calls no API
import { triggerKey } from '@/lib/workspaceClient'
import { NARROW_VIEWPORT_MAX_WIDTH } from '@/lib/useNarrowViewport'
import type { AppState, ViewMode } from '@/store'

const RUNS_HASH_PREFIX = '#/runs/'

export function buildRunsHash(runId: string, nodeId?: string | null): string {
    const encodedRunId = encodeURIComponent(runId)
    if (!nodeId) {
        return `${RUNS_HASH_PREFIX}${encodedRunId}`
    }
    return `${RUNS_HASH_PREFIX}${encodedRunId}/${encodeURIComponent(nodeId)}`
}

/** The URL names a view by its label, where that differs from the view's name in the app. */
const VIEW_LABELS: Partial<Record<ViewMode, string>> = { editor: 'flows', home: 'chats', projects: 'chats' }
const VIEWS: ViewMode[] = ['overview', 'home', 'missions', 'runs', 'triggers', 'editor', 'roles', 'settings']
const viewName = (view: ViewMode) => VIEW_LABELS[view] ?? view

/**
 * Where the app is, as one browser history entry: the view, plus what it has open: the run in
 * Runs, the project page in Chats, the flow in Flows, the trigger in Triggers. Other selections
 * (a mission, a node) change in place. An absent field leaves that selection as it is.
 */
export interface Place {
    view: ViewMode
    runId?: string | null
    nodeId?: string | null
    projectPagePath?: string | null
    /** The project whose flows Flows shows: a flow's name only names it within its project. */
    projectPath?: string | null
    flow?: string | null
    triggerId?: string | null
}

export function placeOf(state: AppState): Place {
    const view = state.viewMode === 'projects' ? 'home' : state.viewMode
    if (view === 'home') return { view, projectPagePath: state.projectPagePath }
    if (view === 'editor') return { view, projectPath: state.activeProjectPath, flow: state.activeFlow }
    if (view === 'triggers') return { view, triggerId: state.triggersSession.selectedTriggerId }
    if (view !== 'runs') return { view }
    const runId = state.runsListSession.selectedRunId
    return { view, runId, nodeId: runId ? state.runDetailSessionsByRunId[runId]?.selectedNodeId ?? null : null }
}

/** Places with the same key are one history entry; a node change only rewrites the entry. */
const placeKey = (place: Place) =>
    [place.view, place.runId, place.projectPagePath, place.projectPath, place.flow, place.triggerId].join('\n')

/**
 * Whether moving between two places adds a history entry. In Triggers only a move from a trigger
 * the list still has to another trigger does: the list picks or drops a trigger by itself when
 * none is picked or the picked one goes, and an entry for that would bounce Back straight off.
 * A phone opens Triggers on the list and picks none itself, so a trigger picked there is the
 * user's, and Back returns to the list.
 */
export function addsEntry(state: AppState, from: Place, to: Place): boolean {
    if (placeKey(from) === placeKey(to)) return false
    if (from.view !== 'triggers' || to.view !== 'triggers') return true
    if (!from.triggerId && to.triggerId && window.innerWidth <= NARROW_VIEWPORT_MAX_WIDTH) return true
    return Boolean(to.triggerId) && state.triggersSession.triggers.some((trigger) => triggerKey(trigger) === from.triggerId)
}

export function placeHash(place: Place): string {
    if (place.runId) return buildRunsHash(place.runId, place.nodeId)
    // A trigger key is its [project, id]; the link names them as a flow's link does.
    const [triggerProject, trigger] = place.triggerId ? JSON.parse(place.triggerId) as [string, string] : []
    const item = place.flow ?? trigger
    const project = place.projectPath ?? triggerProject
    const hash = `#/${viewName(place.view)}${item ? `/${encodeURIComponent(item)}` : ''}`
    return project ? `${hash}?project=${encodeURIComponent(project)}` : hash
}

/** The place a link names, or null when it names none. */
export function parsePlaceHash(hash: string): Place | null {
    const [path, query] = hash.split('?')
    if (!path.startsWith('#/')) return null
    let segments: string[]
    try {
        segments = path.slice(2).split('/').filter(Boolean).map((segment) => decodeURIComponent(segment))
    } catch {
        return null
    }
    const [name, item, nodeId] = segments
    // Older links name a view as the app does (#/editor, #/home); they still open it.
    const view = VIEWS.find((mode) => viewName(mode) === name || mode === name)
    if (!view) return null
    if (view === 'runs' && item) return { view, runId: item, nodeId: nodeId ?? null }
    const projectPath = new URLSearchParams(query).get('project')
    if (view === 'triggers' && item && projectPath) return { view, triggerId: triggerKey({ project_path: projectPath, id: item }) }
    // A flow link without a project opens the flow in the project Flows shows.
    if (view === 'editor' && projectPath) return { view, projectPath, flow: item ?? null }
    if (view === 'editor' && item) return { view, flow: item }
    // A bare view link keeps whichever item was open in that view.
    return { view }
}
