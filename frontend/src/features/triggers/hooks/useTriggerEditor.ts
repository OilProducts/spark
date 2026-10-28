import { useMemo, useRef, useState } from 'react'
import {
    createTriggerValidated,
    deleteTriggerValidated,
    triggerKey,
    updateTriggerValidated,
    type TriggerResponse,
} from '@/lib/workspaceClient'
import { useStore } from '@/store'
import {
    buildTriggerActionPayload,
    buildTriggerSourcePayload,
    createEmptyTriggerForm,
    type TriggerFormState,
    triggerToFormState,
} from '../model/triggerForm'
import { useSettingsNavigationProtection } from '@/features/settings/hooks/useSettingsNavigationProtection'
import { useDialogController } from '@/components/app/dialog-controller'
import { defaultProjectChoice } from '@/features/projects/model/projectChoices'
type UseTriggerEditorArgs = {
    refreshTriggers: () => Promise<void>
    selectedTrigger: TriggerResponse | null
    setError: (value: string | null) => void
    revealWebhookSecret: (triggerId: string, secret: string) => void
    setSelectedTriggerId: (value: string | null) => void
}

const buildProtectedTriggerUpdatePayload = (form: TriggerFormState) => ({
    name: form.name,
    enabled: form.enabled,
    action: {
        flow_name: form.flowName.trim(),
    },
})

export function useTriggerEditor({
    refreshTriggers,
    selectedTrigger,
    setError,
    revealWebhookSecret,
    setSelectedTriggerId,
}: UseTriggerEditorArgs) {
    const { confirm } = useDialogController()
    const pendingRef = useRef(false)
    const [pending, setPending] = useState(false)
    const newTriggerDraft = useStore((state) => state.triggersSession.newTriggerDraft)
    const editTriggerDraftsByTriggerId = useStore((state) => state.triggersSession.editTriggerDraftsByTriggerId)
    const setTriggersSessionNewDraft = useStore((state) => state.setTriggersSessionNewDraft)
    const setTriggersSessionEditDraft = useStore((state) => state.setTriggersSessionEditDraft)
    const updateTriggersSession = useStore((state) => state.updateTriggersSession)

    const selectedTriggerForm = useMemo(
        () => (selectedTrigger ? triggerToFormState(selectedTrigger) : null),
        [selectedTrigger],
    )
    const newTriggerForm = newTriggerDraft.form
    const currentEditDraft = selectedTrigger ? editTriggerDraftsByTriggerId[triggerKey(selectedTrigger)] ?? null : null
    const resolvedEditTriggerForm = currentEditDraft?.form ?? selectedTriggerForm
    const dirty = Boolean(currentEditDraft?.form)
    const externalChange = dirty && currentEditDraft?.expectedRevision !== selectedTrigger?.revision
    useSettingsNavigationProtection(dirty, pending)

    const onCreateTrigger = async () => {
        if (pendingRef.current) return
        pendingRef.current = true
        setPending(true)
        try {
            const created = await createTriggerValidated({
                project_path: newTriggerForm.projectPath,
                name: newTriggerForm.name,
                enabled: newTriggerForm.enabled,
                source_type: newTriggerForm.sourceType,
                action: buildTriggerActionPayload(newTriggerForm),
                source: buildTriggerSourcePayload(newTriggerForm),
            })
            if (created.webhook_secret) {
                revealWebhookSecret(triggerKey(created), created.webhook_secret)
            }
            setTriggersSessionNewDraft({
                form: createEmptyTriggerForm(defaultProjectChoice(useStore.getState().projectRegistry, useStore.getState().activeProjectPath)),
            })
            await refreshTriggers()
            setSelectedTriggerId(triggerKey(created))
            updateTriggersSession({ createFormOpen: false })
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to create trigger.')
        } finally {
            pendingRef.current = false
            setPending(false)
        }
    }

    const onSaveSelectedTrigger = async () => {
        if (pendingRef.current || !selectedTrigger || !resolvedEditTriggerForm) {
            return
        }
        pendingRef.current = true
        setPending(true)
        try {
            const payload = selectedTrigger.protected
                ? buildProtectedTriggerUpdatePayload(resolvedEditTriggerForm)
                : {
                    name: resolvedEditTriggerForm.name,
                    enabled: resolvedEditTriggerForm.enabled,
                    action: { ...buildTriggerActionPayload(resolvedEditTriggerForm), mode: resolvedEditTriggerForm.actionMode },
                    source: buildTriggerSourcePayload(resolvedEditTriggerForm),
                }
            const updated = await updateTriggerValidated(selectedTrigger.project_path, selectedTrigger.id, { ...payload, expected_revision: currentEditDraft?.form ? currentEditDraft.expectedRevision ?? '' : selectedTrigger.revision })
            if (updated.webhook_secret) {
                revealWebhookSecret(triggerKey(selectedTrigger), updated.webhook_secret)
            }
            setTriggersSessionEditDraft(triggerKey(selectedTrigger), null)
            await refreshTriggers()
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to save trigger.')
        } finally {
            pendingRef.current = false
            setPending(false)
        }
    }

    const onDeleteSelectedTrigger = async () => {
        if (pendingRef.current || !selectedTrigger || selectedTrigger.protected) {
            return
        }
        const confirmed = await confirm({
            title: 'Delete trigger?',
            description: `Delete trigger "${selectedTrigger.name}"?`,
            confirmLabel: 'Delete',
            cancelLabel: 'Keep trigger',
            confirmVariant: 'destructive',
        })
        if (!confirmed) {
            return
        }
        if (pendingRef.current) return
        pendingRef.current = true
        setPending(true)
        try {
            await deleteTriggerValidated(selectedTrigger.project_path, selectedTrigger.id, currentEditDraft?.form ? currentEditDraft.expectedRevision ?? '' : selectedTrigger.revision)
            setTriggersSessionEditDraft(triggerKey(selectedTrigger), null)
            setSelectedTriggerId(null)
            await refreshTriggers()
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to delete trigger.')
        } finally {
            pendingRef.current = false
            setPending(false)
        }
    }

    return {
        pending,
        dirty,
        externalChange,
        discard: () => {
            if (pendingRef.current || !selectedTrigger) return
            setTriggersSessionEditDraft(triggerKey(selectedTrigger), null)
            void refreshTriggers()
        },
        editTriggerForm: resolvedEditTriggerForm,
        newTriggerForm,
        onCreateTrigger,
        onDeleteSelectedTrigger,
        onSaveSelectedTrigger,
        setEditTriggerForm: (next: TriggerFormState | null) => {
            if (pendingRef.current || !selectedTrigger) {
                return
            }
            const currentDraft = currentEditDraft
            setTriggersSessionEditDraft(triggerKey(selectedTrigger), {
                triggerId: triggerKey(selectedTrigger),
                expectedRevision: currentDraft?.form ? currentDraft.expectedRevision ?? '' : selectedTrigger.revision,
                form: next,
            })
        },
        setNewTriggerForm: (next: TriggerFormState) => {
            if (pendingRef.current) return
            setTriggersSessionNewDraft({ form: next })
        },
    }
}
