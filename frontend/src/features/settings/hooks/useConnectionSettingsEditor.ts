import { useEffect, useState } from 'react'
import { fetchConnectionSettings, saveConnectionSettings, type ConnectionSettings, type ConnectionSettingsView } from '@/lib/api/settingsApi'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

export function useConnectionSettingsEditor() {
    const [saved, setSaved] = useState<ConnectionSettingsView | null>(null)
    const [draft, setDraft] = useState<ConnectionSettings | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = !!saved && !!draft && JSON.stringify(saved.stored) !== JSON.stringify(draft)
    useSettingsNavigationProtection(dirty, pending)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void fetchConnectionSettings().then((value) => {
                if (cancelled || value.revision === saved?.revision) return
                if (dirty || pending) {
                    setMessage('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                } else { setSaved(value); setDraft(value.stored) }
            }).catch(() => { if (!cancelled) setError('Unable to refresh connection settings.') })
        }
        if (!saved?.revision) refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [saved?.revision, dirty, pending])
    const invalidPort = draft?.server_port != null && (!Number.isInteger(draft.server_port) || draft.server_port < 0 || draft.server_port > 65535)
    const invalidHost = draft?.server_host != null && (!draft.server_host.trim() || /[\s/@?#]/.test(draft.server_host))
    let invalidTarget = false
    if (draft?.client_api_base_url != null) {
        try {
            const target = new URL(draft.client_api_base_url)
            invalidTarget = !['http:', 'https:'].includes(target.protocol) || !!(target.username || target.password || target.search || target.hash)
        } catch { invalidTarget = true }
    }
    const invalid = invalidPort || invalidHost || invalidTarget
    const save = async () => {
        if (!saved || !draft || pending || invalid) return
        setPending(true); setError(''); setMessage('')
        try {
            const value = await saveConnectionSettings(saved.revision, draft)
            setSaved(value); setDraft(value.stored); setMessage('Saved. Restart Spark to apply server binding changes.')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to save connection settings.')
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchConnectionSettings()
            setSaved(value); setDraft(value.stored); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    return { saved, draft, setDraft, pending, error, message, setMessage, dirty, invalidPort, invalidHost, invalidTarget, invalid, save, discard }
}
