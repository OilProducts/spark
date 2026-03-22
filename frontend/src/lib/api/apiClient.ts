import { fetchJsonWithValidation, fetchTextWithValidation } from './shared'

export const WORKSPACE_PREFIX = '/workspace'
export const WORKSPACE_API_PREFIX = `${WORKSPACE_PREFIX}/api`

export function workspaceUrl(path: string): string {
    return `${WORKSPACE_API_PREFIX}${path}`
}

export async function fetchWorkspaceJsonValidated<T>(
    path: string,
    init: RequestInit | undefined,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
): Promise<T> {
    return fetchJsonWithValidation(workspaceUrl(path), init, endpoint, parser)
}

export async function fetchWorkspaceTextValidated<T>(
    path: string,
    init: RequestInit | undefined,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
): Promise<T> {
    return fetchTextWithValidation(workspaceUrl(path), init, endpoint, parser)
}
