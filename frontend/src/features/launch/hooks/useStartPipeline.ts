import { useDialogController } from '@/components/app/dialog-controller'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import {
    ApiHttpError,
    fetchPipelineContinueValidated,
    fetchPipelineStartValidated,
    type PipelineStartResponse,
} from '@/lib/attractorClient'
import { normalizeProjectPath } from '@/lib/projectPaths'
import { fetchProjectMetadataValidated } from '@/lib/workspaceClient'
import { homeSessionStart } from '@/state/homeSessionSlice'
import {
    buildPipelineContinuePayload,
    buildPipelineStartPayload,
    type PipelineContinueFlowSourceMode,
    type RunInitiationFormState,
} from '@/lib/pipelineStartPayload'
import { useStore, workspaceShowing } from '@/store'

export type StartPipelineResult =
    | { status: 'launched'; runId: string | null; queued: boolean }
    | { status: 'cancelled' }

export interface GitPolicyGateResult {
    allowed: boolean
    warning: string | null
}

// A run record's native path (C:\repo) finds the registry entry keyed by its normalized form (C:/repo).
const isRegisteredProject = (projectPath: string) => Boolean(useStore.getState().projectRegistry[normalizeProjectPath(projectPath)])

// The workspace a launch began in: its environment's, until it closes for a switch, or a switch away and back
// starts that environment anew.
const shownWorkspace = () => {
    const environmentId = getActiveEnvironmentId()
    const session = homeSessionStart()
    const showing = workspaceShowing()
    return () => getActiveEnvironmentId() === environmentId && homeSessionStart() === session && workspaceShowing() === showing
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

    const confirmGitPolicyGate = async (projectPath: string, shown: () => boolean): Promise<GitPolicyGateResult> => {
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
            // Not asked in another workspace: the launch it would allow is not there.
            if (!shown()) {
                return { allowed: false, warning }
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
        if (runId && (!projectWasRegistered || isRegisteredProject(projectPath))) {
            setRunsSelectedRunId(runId)
        }
        return { runId, queued }
    }

    // Runs start from the installed flow on disk, by name, in the environment the launch began in. `prepare`, such as
    // the editor's save of that flow, runs first, bound to the same workspace.
    const startFlow = async (
        form: RunInitiationFormState,
        options: { prepare?: () => Promise<void>; onGitPolicyWarning?: (warning: string | null) => void } = {},
    ): Promise<StartPipelineResult> => {
        const environmentId = getActiveEnvironmentId()
        const shown = shownWorkspace()
        const projectWasRegistered = isRegisteredProject(form.projectPath)
        await options.prepare?.()
        const gate = await confirmGitPolicyGate(form.projectPath, shown)
        options.onGitPolicyWarning?.(gate.warning)
        if (!gate.allowed) {
            return { status: 'cancelled' }
        }
        const resolvedWorkingDirectory = form.workingDirectory.trim() || form.projectPath
        const payload = buildPipelineStartPayload({ ...form, workingDirectory: resolvedWorkingDirectory })
        const runData = await fetchPipelineStartValidated(payload, environmentId)
        // Started where the launch began, once its workspace closed, even if it opened again: nothing shown follows it.
        if (!shown()) {
            return { status: 'cancelled' }
        }
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
        const environmentId = getActiveEnvironmentId()
        const shown = shownWorkspace()
        const projectWasRegistered = isRegisteredProject(form.projectPath)
        const gate = await confirmGitPolicyGate(form.projectPath, shown)
        options.onGitPolicyWarning?.(gate.warning)
        if (!gate.allowed) {
            return { status: 'cancelled' }
        }
        const payload = buildPipelineContinuePayload(form, continuation)
        const runData = await fetchPipelineContinueValidated(sourceRunId, payload, environmentId)
        // Started where the launch began, once its workspace closed, even if it opened again: nothing shown follows it.
        if (!shown()) {
            return { status: 'cancelled' }
        }
        return { status: 'launched', ...finalizeLaunch(runData, form.projectPath, projectWasRegistered) }
    }

    return { startFlow, continueFromRun, logUnexpectedLaunchError }
}
