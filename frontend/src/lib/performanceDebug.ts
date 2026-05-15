export const PERFORMANCE_DEBUG_QUERY_PARAM = 'debugPerformance'
export const PERFORMANCE_DEBUG_STORAGE_KEY = 'spark.debug.performance'

const DEBUG_ENABLED_VALUE = '1'

const readWindowSearch = () => {
    if (typeof window === 'undefined') {
        return ''
    }
    return window.location.search
}

const readLocalStorageValue = () => {
    try {
        return globalThis.localStorage?.getItem(PERFORMANCE_DEBUG_STORAGE_KEY) ?? null
    } catch {
        return null
    }
}

export function isPerformanceDebugEnabled(): boolean {
    if (typeof URLSearchParams !== 'undefined') {
        const searchParams = new URLSearchParams(readWindowSearch())
        if (searchParams.get(PERFORMANCE_DEBUG_QUERY_PARAM) === DEBUG_ENABLED_VALUE) {
            return true
        }
    }

    return readLocalStorageValue() === DEBUG_ENABLED_VALUE
}
