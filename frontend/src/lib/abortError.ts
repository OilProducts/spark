// Generic AbortController/fetch cancellation check; lives outside lib/api so
// presentation code can classify aborted loads without importing the API layer.
export function isAbortError(error: unknown): boolean {
    if (error instanceof DOMException) {
        return error.name === 'AbortError'
    }
    return error instanceof Error && error.name === 'AbortError'
}
