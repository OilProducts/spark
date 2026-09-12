import { providerFieldError, type ProviderConnection } from '../services/executionSettings'
import { useEffect, useState } from 'react'
import { fetchProviderSettings, saveProviderSettings, type ProviderSettings, type ProviderSettingsView } from '../services/executionSettings'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

export function useProviderSettingsEditor() {
    const [saved, setSaved] = useState<ProviderSettingsView | null>(null)
    const [draft, setDraft] = useState<ProviderSettings | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = !!saved && !!draft && JSON.stringify(saved.stored) !== JSON.stringify(draft)
    useSettingsNavigationProtection(dirty || pending)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void fetchProviderSettings().then((value) => {
                if (cancelled || value.revision === saved?.revision) return
                if (dirty || pending) {
                    setMessage('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                } else { setSaved(value); setDraft(value.stored) }
            }).catch(() => { if (!cancelled) setError('Unable to refresh provider settings.') })
        }
        if (!saved?.revision) refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [saved?.revision, dirty, pending])
    const invalid = !!draft && Object.values(draft).some((connection) => Object.entries(connection).some(([key, value]) => providerFieldError(key as keyof ProviderConnection, value)))
    const save = async () => {
        if (!saved || !draft || pending || invalid) return
        setPending(true); setError(''); setMessage('')
        try {
            const value = await saveProviderSettings(saved.revision, draft)
            setSaved(value); setDraft(value.stored); setMessage('Saved. Applies to new work.')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to save provider settings.')
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchProviderSettings()
            setSaved(value); setDraft(value.stored); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    return { saved, draft, setDraft, pending, error, message, setMessage, dirty, invalid, save, discard }
}
