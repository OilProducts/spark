import { useEffect, useState } from 'react'
import { useStore } from '@/store'
import { fetchModelSettings } from '@/lib/api/settingsApi'
import { AppNotice } from '@/components/app/app-notice'
import { Button } from '@/components/ui/button'

export function ModelDefaultsController() {
    const [error, setError] = useState('')
    useEffect(() => {
        let cancelled = false
        const refresh = () => {
            void fetchModelSettings().then(({ effective }) => {
                if (cancelled) return
                if (!effective) { setError('Workspace model defaults are invalid. Repair them in Settings.'); return }
                useStore.getState().setUiDefaults({ llm_provider: effective.provider ?? '', llm_profile: effective.llm_profile ?? '',
                    llm_model: effective.model ?? '', reasoning_effort: effective.reasoning_effort ?? '', thinking: effective.thinking == null ? '' : String(effective.thinking), thinking_budget_tokens: effective.thinking_budget_tokens == null ? '' : String(effective.thinking_budget_tokens), reasoning_mode: effective.reasoning_mode == null ? '' : String(effective.reasoning_mode), reasoning_summary: effective.reasoning_summary == null ? '' : String(effective.reasoning_summary) })
                setError('')
            }).catch((error: unknown) => { if (!cancelled) setError(error instanceof Error ? error.message : 'Unable to load workspace model defaults.') })
        }
        refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('spark:model-defaults-retry', refresh)
        window.addEventListener('focus', refresh)
        return () => {
            cancelled = true
            window.removeEventListener('spark:settings-live-event', refresh)
            window.removeEventListener('spark:model-defaults-retry', refresh)
            window.removeEventListener('focus', refresh)
        }
    }, [])
    return error ? (
        <AppNotice
            key={error}
            data-testid="model-defaults-error"
            title="Couldn’t load the default model"
            actions={<Button type="button" variant="outline" size="xs" onClick={() => window.dispatchEvent(new Event('spark:model-defaults-retry'))}>Try again</Button>}
        >
            {error}
        </AppNotice>
    ) : null
}
