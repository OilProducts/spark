import type { AppState } from './store-types'
import { getRunsSelectedRunIdForScope } from './runsSessionScope'

export const selectSelectedRunId = (state: AppState): string | null =>
    getRunsSelectedRunIdForScope(state.runsListSession, state.activeProjectPath)

export const selectSelectedRunSession = (state: AppState) => {
    const runId = selectSelectedRunId(state)
    return runId ? state.runDetailSessionsByRunId[runId] ?? null : null
}
