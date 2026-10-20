export type ContinuationFlowSourceMode = 'snapshot' | 'flow_name'

export interface ContinuationDraft {
    sourceRunId: string
    sourceFlowName: string | null
    sourceWorkingDirectory: string
    sourceModel: string | null
    flowSourceMode: ContinuationFlowSourceMode
    startNodeId: string | null
    workingDir: string
    model: string
    overrideFlowName: string | null
}

export interface LaunchFailureDiagnostics {
    message: string
    failedAt: string
    flowSource: string | null
}

export type LaunchPreviewSource =
    | { kind: 'flow'; flowName: string }
    | { kind: 'runSnapshot'; runId: string; displayName?: string | null }

export interface LaunchTarget {
    flowName: string | null
    /** Runs before launching, e.g. to flush the editor's pending save: runs launch the saved flow by name. */
    prepareLaunch?: () => Promise<void>
    previewSource: LaunchPreviewSource
}
