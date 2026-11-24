import { useEffect, useState } from 'react'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { applyTheme, cachedAppearance, type Appearance } from '@/lib/theme'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

// The appearance is this browser's, kept by this machine's preferences: another environment's
// Settings shows the one in use, keeps a change to it here, and saves its own environment's as it was.
const shownIn = (remote: boolean, view: ClientPreferencesView): ClientPreferencesView =>
    (remote ? { ...view, stored: { ...view.stored, appearance: cachedAppearance() } } : view)

// Counts appearance choices made in any Settings: the latest wins, so an earlier one still saving is dropped.
let appearanceChoice = 0

/** Keeps an appearance chosen in another environment's Settings in this machine's preferences, and applies it. */
async function keepAppearance(appearance: Appearance) {
    const choice = ++appearanceChoice
    const here = await fetchClientPreferences(null)
    if (choice !== appearanceChoice) return
    await saveClientPreferences(here, { ...here.stored, appearance }, null)
    if (choice === appearanceChoice) applyTheme(appearance)
}

export function useClientPreferencesEditor() {
    // Saves go to the environment this Settings is, even after a switch.
    const [environmentId] = useState(getActiveEnvironmentId)
    const remote = environmentId !== null
    // As the server has it; `shown` puts this browser's appearance in it.
    const [saved, setSaved] = useState<ClientPreferencesView | null>(null)
    const shown = saved && shownIn(remote, saved)
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
                } else { setSaved(value); setDraft(shownIn(remote, value).stored) }
            }).catch(() => { if (!cancelled) setError('Unable to refresh client preferences.') })
        }
        if (!saved?.revision) refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [saved?.revision, dirty, pending, remote])
    const invalidWidth = draft?.editor_sidebar_width != null && (!Number.isInteger(draft.editor_sidebar_width) || draft.editor_sidebar_width < 256 || draft.editor_sidebar_width > 560)
    const graphHeight = draft?.run_presentation?.graph_height
    const invalidGraphHeight = graphHeight != null && (!Number.isInteger(graphHeight) || graphHeight < 280 || graphHeight > 960)
    const save = async () => {
        if (!saved || !shown || !draft || pending || invalidWidth || invalidGraphHeight) return
        setPending(true); setError(''); setMessage('')
        try {
            if (draft.appearance !== shown.stored.appearance) {
                if (remote) await keepAppearance(draft.appearance ?? 'system')
                else appearanceChoice += 1
            }
            const value = await saveClientPreferences(saved, remote ? { ...draft, appearance: saved.stored.appearance } : draft, environmentId)
            setSaved(value); setDraft(shownIn(remote, value).stored)
            setMessage('Saved.')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to save client preferences.')
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchClientPreferences()
            setSaved(value); setDraft(shownIn(remote, value).stored); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    return { saved: shown, draft, setDraft, pending, error, message, setMessage, dirty, invalidWidth, invalidGraphHeight, save, discard }
}
