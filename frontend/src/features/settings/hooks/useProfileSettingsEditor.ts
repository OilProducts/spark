import { useEffect, useState } from 'react'
import { profileSettingsRequest, type ProfileSection, type ProfileSettingsView } from '../services/profileSettings'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

export function useProfileSettingsEditor<T>(section: ProfileSection, parse: (value: unknown, endpoint: string) => T, additionalDirty = false) {
    const [saved, setSaved] = useState<ProfileSettingsView<T> | null>(null)
    const [draft, setDraft] = useState<T | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = additionalDirty || (!!saved && JSON.stringify(draft) !== JSON.stringify(saved.stored))
    useSettingsNavigationProtection(dirty || pending)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void profileSettingsRequest(section, parse).then((view) => {
                if (cancelled || saved?.revision === view.revision) return
                if (dirty) setMessage('Profiles changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                else { setSaved(view); setDraft(view.stored); setError('') }
            }).catch((error: unknown) => { if (!cancelled) setError(error instanceof Error ? error.message : 'Unable to load profiles.') })
        }
        if (!saved) refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [section, parse, saved, dirty, pending])
    const save = async () => {
        if (pending || !saved || !draft) return
        setPending(true); setError(''); setMessage('')
        try {
            const view = await profileSettingsRequest(section, parse, { revision: saved.revision, value: draft })
            setSaved(view); setDraft(view.stored); setMessage('Profiles saved. New work uses these settings.')
            window.dispatchEvent(new Event('spark:settings-live-event'))
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to save profiles. Your draft is retained.') }
        finally { setPending(false) }
    }
    const discard = async () => {
        if (pending) return false
        setPending(true)
        try {
            const view = await profileSettingsRequest(section, parse)
            setSaved(view); setDraft(view.stored); setError(''); setMessage('')
            return true
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload profiles.'); return false }
        finally { setPending(false) }
    }
    return { saved, draft, setDraft, pending, dirty, error, message, save, discard }
}
