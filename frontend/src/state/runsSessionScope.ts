import type { RunsListSessionState } from './viewSessionTypes'

const NO_PROJECT_SCOPE_KEY = '__none__'

export function buildRunsScopeKey(scopeMode: 'active' | 'all', activeProjectPath: string | null): string {
    if (scopeMode === 'all') {
        return 'all'
    }
    return `project:${activeProjectPath ?? NO_PROJECT_SCOPE_KEY}`
}

export function getRunsSelectedRunIdForScope(
    runsListSession: Pick<RunsListSessionState, 'scopeMode' | 'selectedRunIdByScopeKey'>,
    activeProjectPath: string | null,
): string | null {
    const scopeKey = buildRunsScopeKey(runsListSession.scopeMode, activeProjectPath)
    return runsListSession.selectedRunIdByScopeKey[scopeKey] ?? null
}
