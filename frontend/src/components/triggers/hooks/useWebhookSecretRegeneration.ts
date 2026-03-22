import { useState, type Dispatch, type SetStateAction } from 'react'
import { updateTriggerValidated, type TriggerResponse } from '@/lib/workspaceClient'

type UseWebhookSecretRegenerationArgs = {
    refreshTriggers: () => Promise<void>
    selectedTrigger: TriggerResponse | null
    setError: (value: string | null) => void
    setRevealedWebhookSecrets: Dispatch<SetStateAction<Record<string, string>>>
}

export function useWebhookSecretRegeneration({
    refreshTriggers,
    selectedTrigger,
    setError,
    setRevealedWebhookSecrets,
}: UseWebhookSecretRegenerationArgs) {
    const [isRegenerating, setIsRegenerating] = useState(false)

    const onRegenerateWebhookSecret = async () => {
        if (!selectedTrigger || selectedTrigger.source_type !== 'webhook') return
        setIsRegenerating(true)
        try {
            const updated = await updateTriggerValidated(selectedTrigger.id, {
                regenerate_webhook_secret: true,
            })
            if (updated.webhook_secret) {
                setRevealedWebhookSecrets((current) => ({ ...current, [selectedTrigger.id]: updated.webhook_secret! }))
            }
            await refreshTriggers()
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to regenerate webhook secret.')
        } finally {
            setIsRegenerating(false)
        }
    }

    return {
        isRegenerating,
        onRegenerateWebhookSecret,
    }
}
