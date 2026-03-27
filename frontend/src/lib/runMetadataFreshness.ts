export type RunMetadataFreshness = 'never' | 'refreshing' | 'fresh' | 'stale'

interface RunMetadataFreshnessInput {
    isLoading: boolean
    lastFetchedAtMs: number | null
    nowMs: number
    staleAfterMs: number
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
