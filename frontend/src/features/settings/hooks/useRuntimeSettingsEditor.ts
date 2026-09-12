import { useEffect, useState } from 'react'
import { fetchRuntimeSettings, saveRuntimeSettings, type RuntimeSettings, type RuntimeSettingsView } from '@/lib/api/settingsApi'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

export function useRuntimeSettingsEditor() {
    const [saved, setSaved] = useState<RuntimeSettingsView | null>(null)
    const [draft, setDraft] = useState<RuntimeSettings | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = !!saved && !!draft && JSON.stringify(saved.stored) !== JSON.stringify(draft)
    useSettingsNavigationProtection(dirty, pending)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void fetchRuntimeSettings().then((value) => {
                if (cancelled || value.revision === saved?.revision) return
                if (dirty || pending) {
                    setMessage('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                } else { setSaved(value); setDraft(value.stored) }
            }).catch(() => { if (!cancelled) setError('Unable to refresh runtime settings.') })
        }
        if (!saved?.revision) refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [saved?.revision, dirty, pending])
    const invalidRoots = draft?.project_roots.some((path) => !path.startsWith('/') && !path.startsWith('~/')) ?? false
    const save = async () => {
        if (!saved || !draft || pending || invalidRoots) return
        setPending(true); setError(''); setMessage('')
        try {
            const value = await saveRuntimeSettings(saved.revision, draft)
            setSaved(value); setDraft(value.stored); setMessage('Saved. Restart Spark to apply runtime changes.')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to save runtime settings.')
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchRuntimeSettings()
            setSaved(value); setDraft(value.stored); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    return { saved, draft, setDraft, pending, error, message, setMessage, dirty, invalidRoots, save, discard }
}
