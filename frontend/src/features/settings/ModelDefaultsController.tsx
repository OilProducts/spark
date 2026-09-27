import { useEffect, useState } from 'react'
import { useStore } from '@/store'
import { loadAndMigrateModelDefaults } from './services/modelDefaultsMigration'

export function ModelDefaultsController() {
    const [error, setError] = useState('')
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            void loadAndMigrateModelDefaults().then(({ effective }) => {
                if (cancelled) return
                if (!effective) { setError('Workspace model defaults are invalid. Repair them in Settings.'); return }
                useStore.getState().setUiDefaults({ llm_provider: effective.provider ?? '', llm_profile: effective.llm_profile ?? '',
                    llm_model: effective.model ?? '', reasoning_effort: effective.reasoning_effort ?? '', thinking: effective.thinking == null ? '' : String(effective.thinking), thinking_budget_tokens: effective.thinking_budget_tokens == null ? '' : String(effective.thinking_budget_tokens), reasoning_mode: effective.reasoning_mode == null ? '' : String(effective.reasoning_mode), reasoning_summary: effective.reasoning_summary == null ? '' : String(effective.reasoning_summary) })
                setError('')
            }).catch((error: unknown) => { if (!cancelled) setError(error instanceof Error ? error.message : 'Unable to load workspace model defaults.') })
        }
        refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => { cancelled = true; window.removeEventListener('spark:settings-live-event', refresh); window.removeEventListener('focus', refresh) }
    }, [])
    return error ? <p role="alert" className="p-2 text-xs text-destructive">{error}</p> : null
}
