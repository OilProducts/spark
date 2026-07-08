export interface RunsRoute {
    runId: string
    nodeId: string | null
}

const RUNS_HASH_PREFIX = '#/runs/'

export function parseRunsHash(hash: string): RunsRoute | null {
    if (!hash.startsWith(RUNS_HASH_PREFIX)) {
        return null
    }
    const segments = hash
        .slice(RUNS_HASH_PREFIX.length)
        .split('/')
        .map((segment) => decodeURIComponent(segment))
        .filter((segment) => segment.length > 0)
    const runId = segments[0]
    if (!runId) {
        return null
    }
    return {
        runId,
        nodeId: segments[1] ?? null,
    }
}

export function buildRunsHash(runId: string, nodeId?: string | null): string {
    const encodedRunId = encodeURIComponent(runId)
    if (!nodeId) {
        return `${RUNS_HASH_PREFIX}${encodedRunId}`
    }
    return `${RUNS_HASH_PREFIX}${encodedRunId}/${encodeURIComponent(nodeId)}`
}

export function isRunsHash(hash: string): boolean {
    return hash.startsWith(RUNS_HASH_PREFIX)
}
