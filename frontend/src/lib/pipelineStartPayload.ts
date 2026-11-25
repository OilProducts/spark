import { isAbsoluteProjectPath, normalizeProjectPath } from '@/lib/projectPaths'

export interface RunInitiationFormState {
    projectPath: string
    flowSource: string
    workingDirectory: string
    model: string | null
    llmProvider?: string | null
    llmProfile?: string | null
    reasoningEffort?: string | null
    launchContext?: Record<string, unknown> | null
    executionProfileId?: string | null
    /** A name for the run, shown in place of the flow's. */
    title?: string | null
}

export interface PipelineStartPayload {
    project_path: string
    working_directory: string
    model: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
    execution_profile_id?: string | null
    launch_context?: Record<string, unknown> | null
    flow_name: string
    title?: string
}

export type PipelineContinueFlowSourceMode = 'snapshot' | 'flow_name'

export interface PipelineContinuePayload {
    start_node: string
    flow_source_mode: PipelineContinueFlowSourceMode
    flow_name?: string | null
    project_path?: string
    working_directory?: string
    model?: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
}

export const resolveExecutionWorkingDirectory = (form: Pick<RunInitiationFormState, 'projectPath' | 'workingDirectory'>): string => {
    const normalizedProjectPath = normalizeProjectPath(form.projectPath)
    // A POSIX project (single leading /) has no UNC shares: there //usr/local is /usr/local.
    const posixProject = /^\/(?!\/)/.test(normalizedProjectPath)
    const workingDirectory = form.workingDirectory.trim()

    if (!workingDirectory) {
        return normalizedProjectPath
    }
    if (isAbsoluteProjectPath(workingDirectory) || !normalizedProjectPath) {
        return normalizeProjectPath(posixProject ? workingDirectory.replace(/^\/+/, '/') : workingDirectory)
    }

    // A relative directory joins the project before normalizing, so its leading .. climbs the project
    // and the project's style decides what a backslash means.
    const separator = normalizedProjectPath.endsWith('/') ? '' : '/'
    return normalizeProjectPath(`${normalizedProjectPath}${separator}${workingDirectory}`)
}

export function buildPipelineStartPayload(
    form: RunInitiationFormState,
): PipelineStartPayload {
    const workingDirectory = resolveExecutionWorkingDirectory(form)
    const payload: PipelineStartPayload = {
        project_path: normalizeProjectPath(form.projectPath),
        working_directory: workingDirectory,
        model: form.model,
        flow_name: form.flowSource,
    }
    if (form.launchContext && Object.keys(form.launchContext).length > 0) {
        payload.launch_context = form.launchContext
    }
    const llmProfile = form.llmProfile?.trim()
    if (llmProfile) {
        payload.llm_profile = llmProfile
    }
    const llmProvider = form.llmProvider?.trim()
    if (llmProvider && llmProvider !== llmProfile) {
        payload.llm_provider = llmProvider
    }
    const reasoningEffort = form.reasoningEffort?.trim()
    if (reasoningEffort) {
        payload.reasoning_effort = reasoningEffort
    }
    const executionProfileId = form.executionProfileId?.trim()
    if (executionProfileId) {
        payload.execution_profile_id = executionProfileId
    }
    const title = form.title?.trim()
    if (title) {
        payload.title = title
    }
    return payload
}

export function buildPipelineContinuePayload(
    form: Pick<RunInitiationFormState, 'projectPath' | 'workingDirectory' | 'model' | 'llmProvider' | 'llmProfile' | 'reasoningEffort'>,
    continuation: {
        startNodeId: string
        flowSourceMode: PipelineContinueFlowSourceMode
        flowName?: string | null
    },
): PipelineContinuePayload {
    const payload: PipelineContinuePayload = {
        start_node: continuation.startNodeId,
        flow_source_mode: continuation.flowSourceMode,
        flow_name: continuation.flowSourceMode === 'flow_name' ? (continuation.flowName || null) : undefined,
        working_directory: resolveExecutionWorkingDirectory(form),
        model: form.model,
    }
    const projectPath = normalizeProjectPath(form.projectPath)
    if (projectPath) {
        payload.project_path = projectPath
    }
    const llmProfile = form.llmProfile?.trim()
    if (llmProfile) {
        payload.llm_profile = llmProfile
    }
    const llmProvider = form.llmProvider?.trim()
    if (llmProvider && llmProvider !== llmProfile) {
        payload.llm_provider = llmProvider
    }
    const reasoningEffort = form.reasoningEffort?.trim()
    if (reasoningEffort) {
        payload.reasoning_effort = reasoningEffort
    }
    return payload
}
