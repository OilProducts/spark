import { useCallback, useEffect, useState } from 'react'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { applyTheme, cachedAppearance, type Appearance } from '@/lib/theme'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

// The theme is this browser's, kept across switches: every environment's Settings shows the one in use,
// and a theme picked there is saved to that environment and applies once the save succeeds, unless the
// save answers after a switch away or a theme picked after it already applied.
// ponytail: other open tabs take a theme change on reload; follow `storage` events if live sync matters.
let appearanceChoice = 0

// A save each environment's Settings refused, with what it was based on: that environment's Settings
// opens with it and the error, to retry against the same revision, until it saves or is discarded.
// ponytail: a Settings opened before the refusal arrives shows it only once reopened; kept for the page's life.
const refusedSaves = new Map<string, { saved: ClientPreferencesView; draft: ClientPreferences; theme: Appearance | null; error: string }>()

export function useClientPreferencesEditor() {
    // Saves go to the environment this Settings is, even after a switch.
    const [environmentId] = useState(getActiveEnvironmentId)
    const scope = environmentId ?? 'local'
    const [refused] = useState(() => refusedSaves.get(scope))
    // As the server has it; its theme is kept as stored, and saved back unless one is picked here.
    const [saved, setSaved] = useState<ClientPreferencesView | null>(refused?.saved ?? null)
    const [draft, setDraft] = useState<ClientPreferences | null>(refused?.draft ?? null)
    const browserTheme = cachedAppearance()
    const [themeChoice, setThemeChoice] = useState<Appearance | null>(refused?.theme ?? null)
    const theme = themeChoice ?? browserTheme
    const [pending, setPending] = useState(false)
    const [error, setError] = useState(refused?.error ?? '')
    const [message, setMessage] = useState('')
    const dirty = !!saved && !!draft && (theme !== browserTheme || JSON.stringify(saved.stored) !== JSON.stringify(draft))
    const dropRefused = useCallback(() => { refusedSaves.delete(scope) }, [scope])
    useSettingsNavigationProtection(dirty, pending, 'settings', dropRefused)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void fetchClientPreferences().then((value) => {
                if (cancelled || value.revision === saved?.revision) return
                if (dirty || pending) {
                    setMessage('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                } else { setSaved(value); setDraft(value.stored) }
            }).catch(() => { if (!cancelled) setError('Unable to refresh client preferences.') })
        }
        if (!saved?.revision) refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [saved?.revision, dirty, pending])
    const invalidWidth = draft?.editor_sidebar_width != null && (!Number.isInteger(draft.editor_sidebar_width) || draft.editor_sidebar_width < 256 || draft.editor_sidebar_width > 560)
    const graphHeight = draft?.run_presentation?.graph_height
    const invalidGraphHeight = graphHeight != null && (!Number.isInteger(graphHeight) || graphHeight < 280 || graphHeight > 960)
    const save = async () => {
        if (!saved || !draft || pending || invalidWidth || invalidGraphHeight) return
        setPending(true); setError(''); setMessage('')
        try {
            const choice = theme !== browserTheme ? ++appearanceChoice : null
            const value = await saveClientPreferences(saved, choice ? { ...draft, appearance: theme } : draft, environmentId)
            if (choice === appearanceChoice && getActiveEnvironmentId() === environmentId) applyTheme(theme)
            dropRefused()
            setSaved(value); setDraft(value.stored); setThemeChoice(null); setMessage('Saved.')
        } catch (error) {
            const message = error instanceof Error ? error.message : 'Unable to save client preferences.'
            refusedSaves.set(scope, { saved, draft, theme: themeChoice, error: message })
            setError(message)
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchClientPreferences()
            dropRefused()
            setSaved(value); setDraft(value.stored); setThemeChoice(null); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    // Picking the theme in use leaves nothing picked, so the form keeps following the browser's.
    const setTheme = (value: Appearance) => setThemeChoice(value === browserTheme ? null : value)
    return { saved, draft, setDraft, theme, setTheme, pending, error, message, setMessage, dirty, invalidWidth, invalidGraphHeight, save, discard }
}
