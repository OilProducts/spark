import { useEffect, useState } from 'react'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { applyTheme, cachedAppearance } from '@/lib/theme'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

// The theme is this browser's, kept across switches: every environment's Settings shows the one in use,
// and a theme chosen there is saved to that environment and applies once the save succeeds, unless a
// theme chosen after it already has.
// ponytail: other open tabs take a theme change on reload; follow `storage` events if live sync matters.
let appearanceChoice = 0
const shownIn = (view: ClientPreferencesView): ClientPreferencesView =>
    ({ ...view, stored: { ...view.stored, appearance: cachedAppearance() } })

export function useClientPreferencesEditor() {
    // Saves go to the environment this Settings is, even after a switch.
    const [environmentId] = useState(getActiveEnvironmentId)
    // As the server has it; `shown` puts this browser's theme in it.
    const [saved, setSaved] = useState<ClientPreferencesView | null>(null)
    const shown = saved && shownIn(saved)
    const [draft, setDraft] = useState<ClientPreferences | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = !!shown && !!draft && JSON.stringify(shown.stored) !== JSON.stringify(draft)
    useSettingsNavigationProtection(dirty, pending)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void fetchClientPreferences().then((value) => {
                if (cancelled || value.revision === saved?.revision) return
                if (dirty || pending) {
                    setMessage('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                } else { setSaved(value); setDraft(shownIn(value).stored) }
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
        if (!saved || !shown || !draft || pending || invalidWidth || invalidGraphHeight) return
        setPending(true); setError(''); setMessage('')
        try {
            // A theme left as it is keeps the one this environment stored.
            const choice = draft.appearance !== shown.stored.appearance ? ++appearanceChoice : null
            const value = await saveClientPreferences(saved, { ...draft, appearance: choice ? draft.appearance : saved.stored.appearance }, environmentId)
            if (choice === appearanceChoice) applyTheme(draft.appearance ?? 'system')
            setSaved(value); setDraft(shownIn(value).stored); setMessage('Saved.')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to save client preferences.')
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchClientPreferences()
            setSaved(value); setDraft(shownIn(value).stored); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    return { saved: shown, draft, setDraft, pending, error, message, setMessage, dirty, invalidWidth, invalidGraphHeight, save, discard }
}
