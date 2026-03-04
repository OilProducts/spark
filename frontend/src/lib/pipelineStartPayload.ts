export interface RunInitiationFormState {
    projectPath: string
    flowSource: string
    workingDirectory: string
    backend: string
    model: string | null
    specArtifactId: string | null
    planArtifactId: string | null
}

export interface PipelineStartPayload {
    flow_content: string
    working_directory: string
    backend: string
    model: string | null
    flow_name: string | null
    spec_id: string | null
    plan_id: string | null
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
    return {
        flow_content: flowContent,
        working_directory: workingDirectory,
        backend: form.backend,
        model: form.model,
        flow_name: form.flowSource || null,
        spec_id: form.specArtifactId,
        plan_id: form.planArtifactId,
    }
}
