import { useEffect, useState } from 'react'
import { fetchAgentSettings, saveAgentSettings, type AgentSettings, type AgentSettingsView } from '../services/executionSettings'
import { useSettingsNavigationProtection } from './useSettingsNavigationProtection'

export function useAgentSettingsEditor() {
    const [saved, setSaved] = useState<AgentSettingsView | null>(null)
    const [draft, setDraft] = useState<AgentSettings | null>(null)
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const [message, setMessage] = useState('')
    const dirty = !!saved && !!draft && JSON.stringify(saved.stored) !== JSON.stringify(draft)
    useSettingsNavigationProtection(dirty || pending)
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            if (pending) return
            void fetchAgentSettings().then((value) => {
                if (cancelled || value.revision === saved?.revision) return
                if (dirty || pending) {
                    setMessage('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                } else { setSaved(value); setDraft(value.stored) }
            }).catch(() => { if (!cancelled) setError('Unable to refresh agent settings.') })
        }
        if (!saved?.revision) refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [saved?.revision, dirty, pending])
    const invalidLimits = !!draft && [draft.tool_output_limits, draft.line_limits].some((limits) => Object.entries(limits).some(([key, value]) => !key.trim() || !Number.isSafeInteger(value) || value < 0))
    const invalidNative = !!draft?.native && Object.entries(draft.native).some(([key, value]) => key !== 'claude_permission_mode' && typeof value === 'string' && !value.trim())
    const invalid = invalidNative || invalidLimits || !!draft && (draft.default_command_timeout_ms <= 0 || draft.max_command_timeout_ms < draft.default_command_timeout_ms || (draft.enable_loop_detection && draft.loop_detection_window <= 0) || ['max_turns', 'max_tool_rounds_per_input', 'default_command_timeout_ms', 'max_command_timeout_ms', 'loop_detection_window', 'max_subagent_depth'].some((key) => !Number.isSafeInteger(draft[key as keyof AgentSettings]) || Number(draft[key as keyof AgentSettings]) < 0))
    const save = async () => {
        if (!saved || !draft || pending || invalid) return
        setPending(true); setError(''); setMessage('')
        try {
            const value = await saveAgentSettings(saved.revision, draft)
            setSaved(value); setDraft(value.stored); setMessage('Saved. Applies to new work.')
        } catch (error) {
            setError(error instanceof Error ? error.message : 'Unable to save agent settings.')
        } finally { setPending(false) }
    }
    const discard = async () => {
        setPending(true)
        try {
            const value = await fetchAgentSettings()
            setSaved(value); setDraft(value.stored); setError(''); setMessage('')
        } catch (error) { setError(error instanceof Error ? error.message : 'Unable to reload settings.') }
        finally { setPending(false) }
    }
    return { saved, draft, setDraft, pending, error, message, setMessage, dirty, invalid, save, discard }
}
