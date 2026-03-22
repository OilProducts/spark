import { useEffect, useState, type Dispatch, type SetStateAction } from 'react'
import {
    createTriggerValidated,
    deleteTriggerValidated,
    updateTriggerValidated,
    type TriggerResponse,
} from '@/lib/workspaceClient'
import {
    buildTriggerActionPayload,
    buildTriggerSourcePayload,
    EMPTY_TRIGGER_FORM,
    type TriggerFormState,
    triggerToFormState,
} from '../triggerForm'

type UseTriggerEditorArgs = {
    refreshTriggers: () => Promise<void>
    selectedTrigger: TriggerResponse | null
    setError: (value: string | null) => void
    setRevealedWebhookSecrets: Dispatch<SetStateAction<Record<string, string>>>
    setSelectedTriggerId: (value: string | null) => void
}

export function useTriggerEditor({
    refreshTriggers,
    selectedTrigger,
    setError,
    setRevealedWebhookSecrets,
    setSelectedTriggerId,
}: UseTriggerEditorArgs) {
    const [newTriggerForm, setNewTriggerForm] = useState<TriggerFormState>(EMPTY_TRIGGER_FORM)
    const [editTriggerForm, setEditTriggerForm] = useState<TriggerFormState | null>(null)

    useEffect(() => {
        if (selectedTrigger) {
            setEditTriggerForm(triggerToFormState(selectedTrigger))
        } else {
            setEditTriggerForm(null)
        }
    }, [selectedTrigger])

    const onCreateTrigger = async () => {
        try {
            const created = await createTriggerValidated({
                name: newTriggerForm.name,
                enabled: newTriggerForm.enabled,
                source_type: newTriggerForm.sourceType,
                action: buildTriggerActionPayload(newTriggerForm),
                source: buildTriggerSourcePayload(newTriggerForm),
            })
            setRevealedWebhookSecrets((current) =>
                created.webhook_secret ? { ...current, [created.id]: created.webhook_secret } : current,
            )
            setNewTriggerForm(EMPTY_TRIGGER_FORM)
            await refreshTriggers()
            setSelectedTriggerId(created.id)
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to create trigger.')
        }
    }

    const onSaveSelectedTrigger = async () => {
        if (!selectedTrigger || !editTriggerForm) return
        try {
            const updated = await updateTriggerValidated(selectedTrigger.id, {
                name: editTriggerForm.name,
                enabled: editTriggerForm.enabled,
                action: buildTriggerActionPayload(editTriggerForm),
                source: selectedTrigger.protected ? undefined : buildTriggerSourcePayload(editTriggerForm),
            })
            setRevealedWebhookSecrets((current) =>
                updated.webhook_secret ? { ...current, [updated.id]: updated.webhook_secret } : current,
            )
            await refreshTriggers()
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to save trigger.')
        }
    }

    const onDeleteSelectedTrigger = async () => {
        if (!selectedTrigger || selectedTrigger.protected) return
        if (!window.confirm(`Delete trigger "${selectedTrigger.name}"?`)) return
        try {
            await deleteTriggerValidated(selectedTrigger.id)
            setSelectedTriggerId(null)
            await refreshTriggers()
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to delete trigger.')
        }
    }

    return {
        editTriggerForm,
        newTriggerForm,
        onCreateTrigger,
        onDeleteSelectedTrigger,
        onSaveSelectedTrigger,
        setEditTriggerForm,
        setNewTriggerForm,
    }
}
