import { ApiHttpError } from '@/lib/attractorClient'
import { nameNodes, problemSentence as sentence } from '@/lib/problemText'

/** Whether a flow failed to load because its project has no flow by that name. */
export const isMissingFlowError = (error: unknown) => error instanceof ApiHttpError && error.status === 404

/** Whether the open flow, in its project, is the one that failed to load. */
export const flowLoadFailed = (
    failure: { flow: string; project: string | null } | null,
    flow: string | null,
    project: string | null,
): failure is { flow: string; project: string | null; missing: boolean; problems?: FlowProblem[] | null } => Boolean(failure && flow && failure.flow === flow && failure.project === project);

/** Whether a flow loaded as files but not as a valid flow, so fixing its files can open it. */
export const isInvalidFlowError = (error: unknown) => error instanceof ApiHttpError && error.status === 422

/** One thing wrong with a flow, and the flow file it is in when it is one file's. */
export interface FlowProblem {
    message: string
    file: string | null
}


/**
 * The problems in a server's flow error, one sentence each, without the save verdict or the
 * error count it leads with.
 */
export function flowProblems(text: string): string[] {
    const detail = text.replace(/^Save blocked by (validation errors|a flow parse error): /, '')
    const listed = /^(?:flow definition )?validation failed with \d+ error\(s\): ([\s\S]+)$/i.exec(detail)
    return (listed ? listed[1].split('; ') : [detail]).filter((problem) => problem.trim()).map(sentence)
}

/** The problems a server's flow error lists in `diagnostics`, each with its file; null when it lists none. */
export function flowProblemsFromDetail(detail: unknown): FlowProblem[] | null {
    const diagnostics = detail && typeof detail === 'object' ? (detail as { diagnostics?: unknown }).diagnostics : null
    if (!Array.isArray(diagnostics)) return null
    type Diagnostic = { message?: unknown; file?: unknown; node_id?: unknown; node_label?: unknown } | null
    // A step a problem names by ID goes by its title, which the flow can't show while it won't load.
    const labels = new Map(diagnostics.flatMap((diagnostic: Diagnostic) => (
        typeof diagnostic?.node_id === 'string' && typeof diagnostic.node_label === 'string' && diagnostic.node_label.trim()
            ? [[diagnostic.node_id, diagnostic.node_label.trim()] as const]
            : []
    )))
    const problems = diagnostics.flatMap((diagnostic: Diagnostic) => (
        typeof diagnostic?.message === 'string' && diagnostic.message.trim()
            ? [{ message: sentence(nameNodes(diagnostic.message, (nodeId) => labels.get(nodeId) ?? nodeId)), file: typeof diagnostic.file === 'string' ? diagnostic.file : null }]
            : []
    ))
    return problems.length > 0 ? problems : null
}

/** A failed flow load's problems: the server's list, else its reason as sentences. */
export function flowLoadProblems(error: unknown): FlowProblem[] | null {
    if (!(error instanceof ApiHttpError)) return null
    const payload = error.payload && typeof error.payload === 'object' ? (error.payload as { detail?: unknown }) : null
    return flowProblemsFromDetail(payload?.detail)
        ?? (error.detail ? flowProblems(error.message).map((message) => ({ message, file: null })) : null)
}
