import { ApiHttpError } from '@/lib/attractorClient'

/** Whether a flow failed to load because its project has no flow by that name. */
export const isMissingFlowError = (error: unknown) => error instanceof ApiHttpError && error.status === 404

/** Whether the open flow, in its project, is the one that failed to load. */
export const flowLoadFailed = (
    failure: { flow: string; project: string | null } | null,
    flow: string | null,
    project: string | null,
): failure is { flow: string; project: string | null; missing: boolean; reason?: string | null } => Boolean(failure && flow && failure.flow === flow && failure.project === project);

/** The server's reason a flow couldn't load, when it gave one (an invalid flow names its errors). */
export const flowLoadReason = (error: unknown) => (error instanceof ApiHttpError && error.detail ? error.message : null)

/** Whether a flow loaded as files but not as a valid flow, so fixing its files can open it. */
export const isInvalidFlowError = (error: unknown) => error instanceof ApiHttpError && error.status === 422

// A sentence that starts with a file or node name ("notify.md: …") keeps its case.
const sentence = (text: string) => {
    const trimmed = text.trim()
    const capitalized = /^[a-z]+\s/.test(trimmed) ? `${trimmed.charAt(0).toUpperCase()}${trimmed.slice(1)}` : trimmed
    return /[.!?]$/.test(capitalized) ? capitalized : `${capitalized}.`
}

/**
 * The problems in a server's flow error, one sentence each, without the save verdict, the flow
 * directory's path or the error count it leads with.
 */
export function flowProblems(text: string): string[] {
    const detail = text
        .replace(/^Save blocked by (validation errors|a flow parse error): /, '')
        .replace(/^Flow directory [^\s:]+: /, '')
    const listed = /^(?:flow definition )?validation failed with \d+ error\(s\): ([\s\S]+)$/i.exec(detail)
    return (listed ? listed[1].split('; ') : [detail]).filter((problem) => problem.trim()).map(sentence)
}
