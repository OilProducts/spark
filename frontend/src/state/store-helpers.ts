import type {
    DiagnosticEntry,
    GraphAttrErrors,
    GraphAttrs,
    ProjectScopedArtifactState,
    ProjectScopedWorkspace,
    RouteState,
    UiDefaults,
    ViewMode,
} from './store-types'
import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'

export const DEFAULT_UI_DEFAULTS: UiDefaults = {
    llm_model: '',
    llm_provider: '',
    reasoning_effort: '',
}

export const UI_DEFAULTS_STORAGE_KEY = 'sparkspawn.ui_defaults'
export const ROUTE_STATE_STORAGE_KEY = 'sparkspawn.ui_route_state'
export const DEFAULT_WORKING_DIRECTORY = './test-app'
export const RECENT_PROJECT_LIMIT = 5
export const VIEW_MODES: ViewMode[] = ['home', 'projects', 'editor', 'execution', 'settings', 'runs']

export const DEFAULT_PROJECT_SCOPED_WORKSPACE: ProjectScopedWorkspace = {
    activeFlow: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    conversationId: null,
    projectEventLog: [],
    specId: null,
    specStatus: 'draft',
    specProvenance: null,
    planId: null,
    planStatus: 'draft',
    planProvenance: null,
}

const GRAPH_FIDELITY_OPTION_SET = new Set<string>([
    'full',
    'truncate',
    'compact',
    'summary:low',
    'summary:medium',
    'summary:high',
])

const STRING_GRAPH_ATTR_KEYS: (keyof GraphAttrs)[] = [
    'sparkspawn.title',
    'sparkspawn.description',
    'sparkspawn.launch_inputs',
    'goal',
    'label',
    'retry_target',
    'fallback_retry_target',
    'default_fidelity',
    'stack.child_dotfile',
    'stack.child_workdir',
    'tool.hooks.pre',
    'tool.hooks.post',
    'ui_default_llm_model',
    'ui_default_llm_provider',
    'ui_default_reasoning_effort',
]

const DEFAULT_MAX_RETRIES_KEY: keyof GraphAttrs = 'default_max_retries'
const LEGACY_DEFAULT_MAX_RETRY_KEY: keyof GraphAttrs = 'default_max_retry'

const modeRequiresActiveProject = (mode: ViewMode) => mode === 'editor' || mode === 'execution'

export const normalizeViewMode = (mode: ViewMode): ViewMode => (mode === 'projects' ? 'home' : mode)

export const resolveViewModeForProjectScope = (mode: ViewMode, activeProjectPath: string | null): ViewMode => {
    const normalizedMode = normalizeViewMode(mode)
    return modeRequiresActiveProject(normalizedMode) && !activeProjectPath ? 'home' : normalizedMode
}

export const pushRecentProjectPath = (recentProjectPaths: string[], projectPath: string | null) => {
    if (!projectPath) {
        return recentProjectPaths
    }
    const deduped = [projectPath, ...recentProjectPaths.filter((path) => path !== projectPath)]
    return deduped.slice(0, RECENT_PROJECT_LIMIT)
}

export const buildDiagnosticMaps = (diagnostics: DiagnosticEntry[]) => {
    const nodeDiagnostics: Record<string, DiagnosticEntry[]> = {}
    const edgeDiagnostics: Record<string, DiagnosticEntry[]> = {}

    diagnostics.forEach((diag) => {
        if (diag.node_id) {
            if (!nodeDiagnostics[diag.node_id]) {
                nodeDiagnostics[diag.node_id] = []
            }
            nodeDiagnostics[diag.node_id].push(diag)
        }
        if (diag.edge && diag.edge.length === 2) {
            const key = `${diag.edge[0]}->${diag.edge[1]}`
            if (!edgeDiagnostics[key]) {
                edgeDiagnostics[key] = []
            }
            edgeDiagnostics[key].push(diag)
        }
    })

    return { nodeDiagnostics, edgeDiagnostics }
}

const isKnownGraphAttrKey = (key: string): key is keyof GraphAttrs => {
    if (key === 'model_stylesheet' || key === DEFAULT_MAX_RETRIES_KEY || key === LEGACY_DEFAULT_MAX_RETRY_KEY) {
        return true
    }
    return STRING_GRAPH_ATTR_KEYS.includes(key as keyof GraphAttrs)
}

const canonicalGraphAttrKey = (key: string): keyof GraphAttrs | string => {
    if (key === LEGACY_DEFAULT_MAX_RETRY_KEY) {
        return DEFAULT_MAX_RETRIES_KEY
    }
    return key
}

export const normalizeGraphAttrValue = (key: keyof GraphAttrs, value: string): string => {
    if (key === 'model_stylesheet') {
        return value
    }
    if (key === DEFAULT_MAX_RETRIES_KEY || key === LEGACY_DEFAULT_MAX_RETRY_KEY) {
        const trimmed = value.trim()
        if (!trimmed) return ''
        if (!/^\d+$/.test(trimmed)) return trimmed
        return `${Math.max(0, parseInt(trimmed, 10))}`
    }
    if (key === 'default_fidelity') {
        return value.trim().toLowerCase()
    }
    if (STRING_GRAPH_ATTR_KEYS.includes(key)) {
        return value.trim()
    }
    return value
}

export const validateGraphAttrValue = (key: keyof GraphAttrs, value: string): string | null => {
    if (key === DEFAULT_MAX_RETRIES_KEY || key === LEGACY_DEFAULT_MAX_RETRY_KEY) {
        if (!value) return null
        if (!/^\d+$/.test(value)) {
            return 'Default max retries must be a non-negative integer.'
        }
        return null
    }
    if (key === 'default_fidelity') {
        if (!value) return null
        if (!GRAPH_FIDELITY_OPTION_SET.has(value)) {
            return 'Default fidelity must be one of: full, truncate, compact, summary:low, summary:medium, summary:high.'
        }
        return null
    }
    return null
}

export const normalizeGraphAttrs = (attrs: GraphAttrs): GraphAttrs => {
    const normalized: Record<string, unknown> = {}
    const entries = Object.entries(attrs) as [string, unknown][]
    entries.forEach(([key, rawValue]) => {
        if (rawValue === undefined || rawValue === null) {
            return
        }
        if (
            key === LEGACY_DEFAULT_MAX_RETRY_KEY
            && attrs[DEFAULT_MAX_RETRIES_KEY] !== undefined
            && attrs[DEFAULT_MAX_RETRIES_KEY] !== null
        ) {
            return
        }
        const canonicalKey = canonicalGraphAttrKey(key)
        if (isKnownGraphAttrKey(key)) {
            normalized[canonicalKey] = normalizeGraphAttrValue(canonicalKey as keyof GraphAttrs, String(rawValue))
            return
        }
        if (typeof rawValue === 'string' || typeof rawValue === 'number' || typeof rawValue === 'boolean') {
            normalized[canonicalKey] = rawValue
            return
        }
        normalized[canonicalKey] = String(rawValue)
    })
    delete normalized[LEGACY_DEFAULT_MAX_RETRY_KEY]
    return normalized as GraphAttrs
}

export const deriveGraphAttrErrors = (attrs: GraphAttrs): GraphAttrErrors => {
    const errors: GraphAttrErrors = {}
    const entries = Object.entries(attrs) as [keyof GraphAttrs, GraphAttrs[keyof GraphAttrs]][]
    entries.forEach(([key, rawValue]) => {
        const value = rawValue === undefined || rawValue === null ? '' : String(rawValue)
        const normalizedValue = normalizeGraphAttrValue(key, value)
        const error = validateGraphAttrValue(key, normalizedValue)
        if (error) {
            errors[key] = error
        }
    })
    return errors
}

export const resolveProjectScopedWorkspace = (
    workspace: Partial<ProjectScopedWorkspace> | undefined,
    projectPath: string | null,
): ProjectScopedWorkspace => {
    const defaultWorkingDir = projectPath || DEFAULT_WORKING_DIRECTORY
    return {
        ...DEFAULT_PROJECT_SCOPED_WORKSPACE,
        ...workspace,
        workingDir: workspace?.workingDir || defaultWorkingDir,
    }
}

export const selectProjectScopedArtifactState = (
    projectScopedWorkspaces: Record<string, ProjectScopedWorkspace>,
    projectPath: string | null,
): ProjectScopedArtifactState | null => {
    if (!projectPath) {
        return null
    }
    const normalizedProjectPath = normalizeProjectPath(projectPath)
    if (!normalizedProjectPath || !isAbsoluteProjectPath(normalizedProjectPath)) {
        return null
    }
    const loadedWorkspace = projectScopedWorkspaces[normalizedProjectPath]
    if (!loadedWorkspace) {
        return null
    }
    const workspace = resolveProjectScopedWorkspace(loadedWorkspace, normalizedProjectPath)
    return {
        conversationId: workspace.conversationId,
        specId: workspace.specId,
        specStatus: workspace.specStatus,
        planId: workspace.planId,
        planStatus: workspace.planStatus,
    }
}

export const DEFAULT_ROUTE_STATE: RouteState = {
    viewMode: 'home',
    activeProjectPath: null,
}

export const loadRouteState = (): RouteState => {
    if (typeof window === 'undefined') {
        return { ...DEFAULT_ROUTE_STATE }
    }
    try {
        const raw = window.localStorage.getItem(ROUTE_STATE_STORAGE_KEY)
        if (!raw) return { ...DEFAULT_ROUTE_STATE }
        const parsed = JSON.parse(raw) as Partial<RouteState>
        const isValidViewMode = parsed.viewMode ? VIEW_MODES.includes(parsed.viewMode) : false
        const requestedViewMode = isValidViewMode ? normalizeViewMode(parsed.viewMode!) : DEFAULT_ROUTE_STATE.viewMode
        const parsedActiveProjectPath = typeof parsed.activeProjectPath === 'string'
            ? normalizeProjectPath(parsed.activeProjectPath)
            : null
        const restoredActiveProjectPath =
            parsedActiveProjectPath && isAbsoluteProjectPath(parsedActiveProjectPath)
                ? parsedActiveProjectPath
                : null
        const parsedRouteState: RouteState = {
            viewMode: requestedViewMode,
            activeProjectPath: restoredActiveProjectPath,
        }
        return {
            ...parsedRouteState,
            viewMode: resolveViewModeForProjectScope(parsedRouteState.viewMode, parsedRouteState.activeProjectPath),
        }
    } catch {
        return { ...DEFAULT_ROUTE_STATE }
    }
}

export const saveRouteState = (state: RouteState) => {
    if (typeof window === 'undefined') return
    try {
        window.localStorage.setItem(ROUTE_STATE_STORAGE_KEY, JSON.stringify(state))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
}

export const loadUiDefaults = (): UiDefaults => {
    if (typeof window === 'undefined') {
        return { ...DEFAULT_UI_DEFAULTS }
    }
    try {
        const raw = window.localStorage.getItem(UI_DEFAULTS_STORAGE_KEY)
        if (!raw) return { ...DEFAULT_UI_DEFAULTS }
        const parsed = JSON.parse(raw) as Partial<UiDefaults>
        return {
            ...DEFAULT_UI_DEFAULTS,
            ...parsed,
        }
    } catch {
        return { ...DEFAULT_UI_DEFAULTS }
    }
}

export const saveUiDefaults = (defaults: UiDefaults) => {
    if (typeof window === 'undefined') return
    try {
        window.localStorage.setItem(UI_DEFAULTS_STORAGE_KEY, JSON.stringify(defaults))
    } catch {
        // Ignore storage failures (private mode, quota, etc.)
    }
}
