import { ApiHttpError } from '@/lib/attractorClient'

/** Whether a flow failed to load because its project has no flow by that name. */
export const isMissingFlowError = (error: unknown) => error instanceof ApiHttpError && error.status === 404

/** Whether the open flow, in its project, is the one that failed to load. */
export const flowLoadFailed = (
    failure: { flow: string; project: string | null } | null,
    flow: string | null,
    project: string | null,
): failure is { flow: string; project: string | null; missing: boolean } => Boolean(failure && flow && failure.flow === flow && failure.project === project);
