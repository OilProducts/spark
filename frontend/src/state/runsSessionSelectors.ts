import type { AppState } from './store-types'

export const selectSelectedRunId = (state: AppState): string | null => state.runsListSession.selectedRunId

export const selectSelectedRunSession = (state: AppState) => {
    const runId = selectSelectedRunId(state)
    return runId ? state.runDetailSessionsByRunId[runId] ?? null : null
}
