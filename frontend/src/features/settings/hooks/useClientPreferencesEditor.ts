import { useEffect, useState } from 'react'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

export function useClientPreferencesEditor() {
    const [saved, setSaved] = useState<ClientPreferencesView | null>(null)
    const [draft, setDraft] = useState<ClientPreferences | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = !!saved && !!draft && JSON.stringify(saved.stored) !== JSON.stringify(draft)
    useSettingsNavigationProtection(dirty, pending)
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
    const ratio = draft?.home_sidebar_primary_split_ratio
    const invalidSplit = ratio != null && (!Number.isFinite(ratio) || ratio < 0 || ratio > 1)
    const graphHeight = draft?.run_presentation?.graph_height
    const invalidGraphHeight = graphHeight != null && (!Number.isInteger(graphHeight) || graphHeight < 280 || graphHeight > 960)
    const save = async () => {
        if (!saved || !draft || pending || invalidWidth || invalidSplit || invalidGraphHeight) return
        setPending(true); setError(''); setMessage('')
        try {
            const value = await saveClientPreferences(saved, draft)
            setSaved(value); setDraft(value.stored); setMessage('Saved.')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to save client preferences.')
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchClientPreferences()
            setSaved(value); setDraft(value.stored); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    return { saved, draft, setDraft, pending, error, message, setMessage, dirty, invalidWidth, invalidSplit, invalidGraphHeight, save, discard }
}
