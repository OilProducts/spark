import { useDialogController } from '@/components/app/dialog-controller'
import {
    ApiHttpError,
    fetchPipelineContinueValidated,
    fetchPipelineStartValidated,
    type PipelineStartResponse,
} from '@/lib/attractorClient'
import { fetchProjectMetadataValidated } from '@/lib/workspaceClient'
import {
    buildPipelineContinuePayload,
    buildPipelineStartPayload,
    type PipelineContinueFlowSourceMode,
    type RunInitiationFormState,
} from '@/lib/pipelineStartPayload'
import { useStore } from '@/store'

export type StartPipelineResult =
    | { status: 'launched'; runId: string | null; queued: boolean }
    | { status: 'cancelled' }

export interface GitPolicyGateResult {
    allowed: boolean
    warning: string | null
}

const logUnexpectedLaunchError = (error: unknown) => {
    if (error instanceof ApiHttpError) {
        return
    }
    console.error(error)
}

export function launchErrorMessage(error: unknown): string {
    return error instanceof ApiHttpError && error.detail
        ? error.detail
        : error instanceof Error
            ? error.message
            : 'Failed to start pipeline run.'
}

export function useStartPipeline() {
    const { confirm } = useDialogController()
    const setRunsSelectedRunId = useStore((state) => state.setRunsSelectedRunId)

    const confirmGitPolicyGate = async (projectPath: string): Promise<GitPolicyGateResult> => {
        if (!projectPath) {
            return { allowed: true, warning: null }
        }

        try {
            await fetchProjectMetadataValidated(projectPath)
            return { allowed: true, warning: null }
        } catch (err) {
            const warning = 'Unable to verify project Git state before run start.'
            if (err instanceof ApiHttpError && err.detail) {
                console.warn(err.detail)
            }
            const allowed = await confirm({
                title: 'Unable to verify Git state',
                description: `${warning} Continue with run start anyway?`,
                confirmLabel: 'Continue',
                cancelLabel: 'Cancel',
            })
            return { allowed, warning }
        }
    }

    // A run launched in a project removed meanwhile is not selected.
    const finalizeLaunch = (runData: PipelineStartResponse, projectPath: string, projectWasRegistered: boolean): { runId: string | null; queued: boolean } => {
        if (runData.status !== 'started' && runData.status !== 'queued') {
            const reason = runData.error || runData.status || 'Unknown run error'
            throw new Error(`Run not started: ${reason}`)
        }
        const runId = typeof runData.pipeline_id === 'string' ? runData.pipeline_id : null
        const queued = runData.status === 'queued'
        if (runId && (!projectWasRegistered || useStore.getState().projectRegistry[projectPath])) {
            setRunsSelectedRunId(runId)
        }
        return { runId, queued }
    }

    const startFromFlowContent = async (
        form: RunInitiationFormState,
        flowContent: string,
        options: { onGitPolicyWarning?: (warning: string | null) => void } = {},
    ): Promise<StartPipelineResult> => {
        const projectWasRegistered = Boolean(useStore.getState().projectRegistry[form.projectPath])
        const gate = await confirmGitPolicyGate(form.projectPath)
        options.onGitPolicyWarning?.(gate.warning)
        if (!gate.allowed) {
            return { status: 'cancelled' }
        }
        const resolvedWorkingDirectory = form.workingDirectory.trim() || form.projectPath
        const payload = buildPipelineStartPayload(
            { ...form, workingDirectory: resolvedWorkingDirectory },
            flowContent,
        )
        const runData = await fetchPipelineStartValidated(payload)
        return { status: 'launched', ...finalizeLaunch(runData, form.projectPath, projectWasRegistered) }
    }

    const continueFromRun = async (
        sourceRunId: string,
        form: Pick<RunInitiationFormState, 'projectPath' | 'workingDirectory' | 'model' | 'llmProvider' | 'llmProfile' | 'reasoningEffort'>,
        continuation: {
            startNodeId: string
            flowSourceMode: PipelineContinueFlowSourceMode
            flowName?: string | null
        },
        options: { onGitPolicyWarning?: (warning: string | null) => void } = {},
    ): Promise<StartPipelineResult> => {
        const projectWasRegistered = Boolean(useStore.getState().projectRegistry[form.projectPath])
        const gate = await confirmGitPolicyGate(form.projectPath)
        options.onGitPolicyWarning?.(gate.warning)
        if (!gate.allowed) {
            return { status: 'cancelled' }
        }
        const payload = buildPipelineContinuePayload(form, continuation)
        const runData = await fetchPipelineContinueValidated(sourceRunId, payload)
        return { status: 'launched', ...finalizeLaunch(runData, form.projectPath, projectWasRegistered) }
    }

    return { startFromFlowContent, continueFromRun, logUnexpectedLaunchError }
}
