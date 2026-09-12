import { useEffect, useState } from 'react'
import { fetchModelSettings, saveModelSettings, type ModelSettings, type ModelSettingsView } from '@/lib/api/settingsApi'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

export function useModelSettingsEditor(projectPath?: string) {
    const [saved, setSaved] = useState<ModelSettingsView | null>(null)
    const [draft, setDraft] = useState<ModelSettings | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = !!saved && JSON.stringify(saved.stored ?? (projectPath ? null : saved.effective)) !== JSON.stringify(draft)
    useSettingsNavigationProtection(dirty, pending)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void fetchModelSettings(projectPath).then((value) => {
                if (cancelled || (value.revision === saved?.revision && JSON.stringify(value.effective) === JSON.stringify(saved?.effective))) return
                if (dirty || pending) setMessage('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                else {
                    setSaved(value); setDraft(value.stored ?? (projectPath ? null : value.effective)); setError('')
                }
            }).catch(() => { if (!cancelled) setError('Unable to load model settings.') })
        }
        refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [projectPath, saved?.revision, saved?.effective, dirty, pending])
    const save = async () => {
        if (!saved || pending || (!projectPath && !draft)) return
        setPending(true); setError(''); setMessage('')
        try {
            const value = await saveModelSettings(saved.revision, draft, projectPath)
            setSaved(value); setDraft(value.stored ?? (projectPath ? null : value.effective)); setMessage('Saved. Applies to the next message.')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to save model settings.') }
        finally { setPending(false) }
    }
    const discard = async () => {
        if (pending) return
        setPending(true)
        try {
            const value = await fetchModelSettings(projectPath)
            setSaved(value); setDraft(value.stored ?? (projectPath ? null : value.effective)); setMessage(''); setError('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload model settings.') }
        finally { setPending(false) }
    }
    return { saved, draft, setDraft, pending, error, message, dirty, save, discard }
}
