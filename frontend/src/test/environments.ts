import { EXPECTED_PROTOCOL_VERSION, EXPECTED_UI_API_VERSION, selectEnvironment } from '@/lib/activeEnvironment'

const descriptor = (environmentId: string) => ({
    environment_id: environmentId,
    display_name: environmentId,
    protocol_version: EXPECTED_PROTOCOL_VERSION,
    ui_api_version: EXPECTED_UI_API_VERSION,
    spark_version: '',
})

/**
 * Makes `environmentId` (`null`: this machine) the active environment, answering the version checks
 * the switch makes itself once `checked` settles; every other request goes to the test's own `fetch`.
 * Open editors let the switch through: what they ask before one is the switcher's own tests' concern.
 */
export async function switchEnvironment(environmentId: string | null, checked?: Promise<unknown>) {
    const testFetch = globalThis.fetch
    const dispatch = window.dispatchEvent
    window.dispatchEvent = (event: Event) => event.type === 'spark:before-navigation' || dispatch.call(window, event)
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url
        if (!url.endsWith('/environment')) return testFetch(input, init)
        await checked
        const id = url === '/environment' ? 'local' : environmentId ?? 'local'
        return new Response(JSON.stringify(descriptor(id)), { headers: { 'Content-Type': 'application/json' } })
    }) as typeof fetch
    try {
        if (!(await selectEnvironment(environmentId))) throw new Error(`Could not switch to ${environmentId ?? 'local'}.`)
    } finally {
        globalThis.fetch = testFetch
        window.dispatchEvent = dispatch
    }
}

/** A promise with its resolve and reject in hand, to finish a request when a test says so. */
export function deferred<T>() {
    let resolve!: (value: T) => void
    let reject!: (error: unknown) => void
    const promise = new Promise<T>((res, rej) => {
        resolve = res
        reject = rej
    })
    return { promise, resolve, reject }
}
