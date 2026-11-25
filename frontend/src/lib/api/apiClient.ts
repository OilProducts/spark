import { environmentProxyPrefix, getActiveEnvironmentId } from '../activeEnvironment'
import { fetchJsonWithValidation } from './shared'

const WORKSPACE_PREFIX = '/workspace'
const WORKSPACE_API_PREFIX = `${WORKSPACE_PREFIX}/api`

/** A workspace API URL in `environmentId` (`null`: this machine), by default the active environment. */
export function workspaceUrl(path: string, environmentId: string | null = getActiveEnvironmentId()): string {
    return `${environmentProxyPrefix(environmentId)}${WORKSPACE_API_PREFIX}${path}`
}

export function workspaceLiveEventsUrl(params: URLSearchParams): string {
    const query = params.toString()
    return workspaceUrl(`/live/events${query ? `?${query}` : ''}`)
}

/** A request that must reach the environment it started in passes that `environmentId`. */
export async function fetchWorkspaceJsonValidated<T>(
    path: string,
    init: RequestInit | undefined,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
    environmentId: string | null = getActiveEnvironmentId(),
): Promise<T> {
    return fetchJsonWithValidation(workspaceUrl(path, environmentId), init, endpoint, parser)
}
