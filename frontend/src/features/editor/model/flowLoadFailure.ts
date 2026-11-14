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
