export type RunMetadataFreshness = 'never' | 'refreshing' | 'fresh' | 'stale'

interface RunMetadataFreshnessInput {
    isLoading: boolean
    lastFetchedAtMs: number | null
    nowMs: number
    staleAfterMs: number
}

interface RunMetadataLastUpdatedInput {
    lastFetchedAtMs: number | null
    nowMs: number
}

export const RUN_METADATA_STALE_AFTER_MS = 30_000

export const computeRunMetadataFreshness = ({
    isLoading,
    lastFetchedAtMs,
    nowMs,
    staleAfterMs,
}: RunMetadataFreshnessInput): RunMetadataFreshness => {
    if (isLoading) return 'refreshing'
    if (typeof lastFetchedAtMs !== 'number') return 'never'
    if (nowMs - lastFetchedAtMs >= staleAfterMs) return 'stale'
    return 'fresh'
}

export const formatRunMetadataLastUpdated = ({ lastFetchedAtMs, nowMs }: RunMetadataLastUpdatedInput): string => {
    if (typeof lastFetchedAtMs !== 'number') return 'Never refreshed'
    const elapsedMs = Math.max(0, nowMs - lastFetchedAtMs)
    const elapsedSeconds = Math.floor(elapsedMs / 1000)
    return `Updated ${elapsedSeconds}s ago`
}
