/** Said when a request never reached Spark, in place of the browser's bare "Failed to fetch". */
export const UNREACHABLE_MESSAGE = 'Couldn’t reach Spark. Check that it is still running; this retries once it is back.'

/** `fetch` that reports an unreachable server in words; aborts and HTTP errors pass through. */
export async function fetchSpark(url: string, init?: RequestInit): Promise<Response> {
    try {
        return await fetch(url, init)
    } catch (error) {
        if (error instanceof TypeError) throw new Error(UNREACHABLE_MESSAGE, { cause: error })
        throw error
    }
}
