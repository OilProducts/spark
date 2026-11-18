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
 * Runs, the project page in Chats, the flow in Flows, the trigger in Triggers, the mission in
 * Missions, the role in Roles. Other selections (a node) change in place. An absent field leaves that selection as it is.
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
    /** The open mission, as its `missionKey`. */
    missionId?: string | null
    /** The open role, as its `roleKey`. */
    roleId?: string | null
}

/** A mission's key in a place: its id only names it within its project. */
export const missionKey = (mission: { projectPath: string; id: string } | null) => (mission ? JSON.stringify([mission.projectPath, mission.id]) : null)
/** A role's key in a place: its name only names it within its project. */
export const roleKey = (role: { projectPath: string; name: string } | null) => (role ? JSON.stringify([role.projectPath, role.name]) : null)

export function placeOf(state: AppState): Place {
    const view = state.viewMode === 'projects' ? 'home' : state.viewMode
    if (view === 'home') return { view, projectPagePath: state.projectPagePath }
    if (view === 'editor') return { view, projectPath: state.activeProjectPath, flow: state.activeFlow }
    if (view === 'triggers') return { view, triggerId: state.triggersSession.selectedTriggerId }
    if (view === 'missions') return { view, missionId: missionKey(state.selectedMission) }
    if (view === 'roles') return { view, roleId: roleKey(state.selectedRole) }
    if (view !== 'runs') return { view }
    const runId = state.runsListSession.selectedRunId
    return { view, runId, nodeId: runId ? state.runDetailSessionsByRunId[runId]?.selectedNodeId ?? null : null }
}

/** Places with the same key are one history entry; a node change only rewrites the entry. */
const placeKey = (place: Place) =>
    [place.view, place.runId, place.projectPagePath, place.projectPath, place.flow, place.triggerId, place.missionId, place.roleId].join('\n')

/**
 * Whether moving between two places adds a history entry. In Runs, the run a desktop opens by itself
 * (when none is open, in a missing one's place, or in place of one it opened before the history
 * arrived) doesn't; one you open does. In Triggers only a move from a trigger
 * the list still has to another trigger does: the list picks or drops a trigger by itself when
 * none is picked or the picked one goes, and an entry for that would bounce Back straight off.
 * A phone opens Triggers on the list and picks none itself, so a trigger picked there is the
 * user's, and Back returns to the list.
 */
export function addsEntry(state: AppState, from: Place, to: Place): boolean {
    if (placeKey(from) === placeKey(to)) return false
    // A run that isn't there closes itself; its link's entry is rewritten, so Back doesn't reopen it.
    if (from.runId && !to.runId && to.view === 'runs' && state.runsListSession.missingRunId === from.runId) return false
    // A desktop's Runs, like its Triggers, opens a run by itself; any run you open clears autoOpenedRunId,
    // so it never takes the place of one you opened.
    if (from.view === 'runs' && to.view === 'runs' && to.runId && to.runId === state.runsListSession.autoOpenedRunId) return false
    if (from.view !== 'triggers' || to.view !== 'triggers') return true
    if (!from.triggerId && to.triggerId && window.innerWidth <= NARROW_VIEWPORT_MAX_WIDTH) return true
    return Boolean(to.triggerId) && state.triggersSession.triggers.some((trigger) => triggerKey(trigger) === from.triggerId)
}

export function placeHash(place: Place): string {
    if (place.runId) return buildRunsHash(place.runId, place.nodeId)
    // A trigger's, a mission's and a role's key is its [project, id]; the link names them as a flow's link does.
    const [triggerProject, trigger] = place.triggerId ? JSON.parse(place.triggerId) as [string, string] : []
    const [missionProject, mission] = place.missionId ? JSON.parse(place.missionId) as [string, string] : []
    const [roleProject, role] = place.roleId ? JSON.parse(place.roleId) as [string, string] : []
    const item = place.flow ?? trigger ?? mission ?? role
    const project = place.projectPath ?? triggerProject ?? missionProject ?? roleProject
    const hash = `#/${viewName(place.view)}${item ? `/${encodeURIComponent(item)}` : ''}`
    return project ? `${hash}?project=${encodeURIComponent(project)}` : hash
}

/**
 * The place a link names, or null when it names none. A trigger, mission or role link without a
 * project names it in `projectInView`, the project the app shows, as a flow link without one does.
 */
export function parsePlaceHash(hash: string, projectInView: string | null = null): Place | null {
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
    const projectPath = new URLSearchParams(query).get('project') ?? (view === 'editor' ? null : projectInView)
    if (view === 'triggers' && item && projectPath) return { view, triggerId: triggerKey({ project_path: projectPath, id: item }) }
    if (view === 'missions' && item && projectPath) return { view, missionId: missionKey({ projectPath, id: item }) }
    if (view === 'roles' && item && projectPath) return { view, roleId: roleKey({ projectPath, name: item }) }
    // A flow link without a project opens the flow in the project Flows shows.
    if (view === 'editor' && projectPath) return { view, projectPath, flow: item ?? null }
    if (view === 'editor' && item) return { view, flow: item }
    // A bare view link keeps whichever item was open in that view.
    return { view }
}
