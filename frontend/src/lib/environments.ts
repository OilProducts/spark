import { isTauri } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'

// Another Spark environment this page can open, kept in this page's local storage.
export type SavedEnvironment = { origin: string, expected_id: string, label: string }

const STORAGE_KEY = 'spark.environments'

/** The origin of a bare http(s) URL (no credentials, path, query or fragment), or null. */
export function normalizeOrigin(input: string): string | null {
    let url: URL
    try { url = new URL(input.trim()) } catch { return null }
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null
    if (url.username || url.password || url.pathname !== '/' || url.search || url.hash) return null
    return url.origin
}

export function loadEnvironments(): SavedEnvironment[] {
    try {
        const value: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]')
        if (!Array.isArray(value)) return []
        return value.filter((entry): entry is SavedEnvironment => typeof entry?.origin === 'string' && normalizeOrigin(entry.origin) === entry.origin
            && typeof entry.expected_id === 'string' && typeof entry.label === 'string')
    } catch { return [] }
}

export function saveEnvironments(entries: SavedEnvironment[]) {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries))
}

/**
 * Opens a saved environment only if its origin still serves the expected environment ID.
 * The desktop hands the origin to the system browser; a browser tab navigates to it.
 */
export async function openEnvironment({ origin, expected_id }: SavedEnvironment, navigate = (url: string) => window.location.assign(url)) {
    let id: unknown
    try {
        const response = await fetch(`${origin}/environment`, { credentials: 'omit', redirect: 'error' })
        if (!response.ok) throw new Error(`HTTP ${response.status}`)
        id = (await response.json() as { environment_id?: unknown } | null)?.environment_id
    } catch {
        throw new Error(`Could not check ${origin}. Is Spark running there?`)
    }
    if (id !== expected_id) throw new Error(`${origin} is not environment ${expected_id} (it reports ${String(id)}). It was not opened.`)
    if (isTauri()) await openUrl(`${origin}/`)
    else navigate(`${origin}/`)
}
