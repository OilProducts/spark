export type Appearance = 'system' | 'light' | 'dark'

const STORAGE_KEY = 'spark.appearance'
const CHANGE_EVENT = 'spark:theme-change'
let stopFollowingSystem = () => {}

export function resolveTheme(preference: Appearance, systemPrefersDark: boolean): 'light' | 'dark' {
    return preference === 'system' ? (systemPrefersDark ? 'dark' : 'light') : preference
}

export function applyTheme(preference: Appearance) {
    stopFollowingSystem()
    const media = window.matchMedia?.('(prefers-color-scheme: dark)')
    const update = () => {
        const theme = resolveTheme(preference, media?.matches ?? false)
        document.documentElement.classList.toggle('dark', theme === 'dark')
        document.documentElement.style.colorScheme = theme
        window.dispatchEvent(new Event(CHANGE_EVENT))
    }
    update()
    if (preference === 'system' && media) {
        media.addEventListener('change', update)
        stopFollowingSystem = () => media.removeEventListener('change', update)
    } else stopFollowingSystem = () => {}
    try { localStorage.setItem(STORAGE_KEY, preference) } catch { /* storage unavailable; server value still applies */ }
}

/** Last applied preference, used before the server value arrives to avoid a flash of the wrong theme. */
export function cachedAppearance(): Appearance {
    try {
        const value = localStorage.getItem(STORAGE_KEY)
        if (value === 'light' || value === 'dark') return value
    } catch { /* storage unavailable */ }
    return 'system'
}

export function subscribeTheme(onChange: () => void) {
    window.addEventListener(CHANGE_EVENT, onChange)
    return () => window.removeEventListener(CHANGE_EVENT, onChange)
}
