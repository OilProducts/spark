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
    projectDefaultExecutionProfileId?: string | null
}

export interface PipelineStartPayload {
    flow_content: string
    working_directory: string
    model: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
    execution_profile_id?: string | null
    project_default_execution_profile_id?: string | null
    launch_context?: Record<string, unknown> | null
    flow_name: string | null
}

export type PipelineContinueFlowSourceMode = 'snapshot' | 'flow_name'

export interface PipelineContinuePayload {
    start_node: string
    flow_source_mode: PipelineContinueFlowSourceMode
    flow_name?: string | null
    working_directory?: string
    model?: string | null
    llm_provider?: string | null
    llm_profile?: string | null
    reasoning_effort?: string | null
}

const normalizePath = (value: string): string => {
    const trimmed = value.trim()
    if (!trimmed) return ''

    const slashNormalized = trimmed.replace(/\\/g, '/').replace(/\/{2,}/g, '/')
    const windowsPrefixMatch = slashNormalized.match(/^[A-Za-z]:\//)
    const prefix = slashNormalized.startsWith('/') ? '/' : windowsPrefixMatch ? windowsPrefixMatch[0] : ''
    const rawBody = prefix ? slashNormalized.slice(prefix.length) : slashNormalized
    const parts = rawBody.split('/').filter((part) => part.length > 0)
    const segments: string[] = []
    for (const part of parts) {
        if (part === '.') {
            continue
        }
        if (part === '..') {
            if (segments.length > 0) {
                segments.pop()
            }
            continue
        }
        segments.push(part)
    }
    const normalizedBody = segments.join('/')
    if (!normalizedBody) {
        if (prefix === '/') {
            return '/'
        }
        return prefix || normalizedBody
    }
    return `${prefix}${normalizedBody}`
}

const isAbsolutePath = (value: string): boolean => value.startsWith('/') || /^[A-Za-z]:\//.test(value)

export const resolveExecutionWorkingDirectory = (form: Pick<RunInitiationFormState, 'projectPath' | 'workingDirectory'>): string => {
    const normalizedProjectPath = normalizePath(form.projectPath)
    const normalizedWorkingDirectory = normalizePath(form.workingDirectory)

    if (!normalizedWorkingDirectory) {
        return normalizedProjectPath
    }
    if (isAbsolutePath(normalizedWorkingDirectory) || !normalizedProjectPath) {
        return normalizedWorkingDirectory
    }

    const separator = normalizedProjectPath.endsWith('/') ? '' : '/'
    return normalizePath(`${normalizedProjectPath}${separator}${normalizedWorkingDirectory}`)
}

export function buildPipelineStartPayload(
    form: RunInitiationFormState,
    flowContent: string,
): PipelineStartPayload {
    const workingDirectory = resolveExecutionWorkingDirectory(form)
    const payload: PipelineStartPayload = {
        flow_content: flowContent,
        working_directory: workingDirectory,
        model: form.model,
        flow_name: form.flowSource || null,
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
    const projectDefaultExecutionProfileId = form.projectDefaultExecutionProfileId?.trim()
    if (!executionProfileId && projectDefaultExecutionProfileId) {
        payload.project_default_execution_profile_id = projectDefaultExecutionProfileId
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
