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
    validateTriggerForm,
} from '../model/triggerForm'
import { useSettingsNavigationProtection } from '@/features/settings/hooks/useSettingsNavigationProtection'
import { useFlowLaunchMetadata } from '@/features/launch'
import type { LaunchInputDefinition } from '@/lib/flowContracts'
import { useDialogController } from '@/components/app/dialog-controller'
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

const NO_INPUTS: LaunchInputDefinition[] = []

/**
 * The launch inputs of the flow a form starts: none when it starts no flow (or its flow can't be
 * read, which the server then judges), null while they load or after the request for them failed,
 * which `retry` asks again.
 */
function useFlowInputs(form: Pick<TriggerFormState, 'actionMode' | 'flowName' | 'projectPath'> | null | undefined): { inputs: LaunchInputDefinition[] | null; retry?: () => void } {
    const flowName = form?.actionMode === 'static' ? form.flowName.trim() : ''
    const flow = useFlowLaunchMetadata(flowName || null, form?.projectPath || null)
    if (!flowName) return { inputs: NO_INPUTS }
    if (!flow || flow.name !== flowName) return { inputs: null }
    // Without the inputs, a required one can't be checked, so saving waits for them.
    return flow.retry ? { inputs: null, retry: flow.retry } : { inputs: flow.inputs ?? NO_INPUTS }
}

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

    const savedFlowInputs = useFlowInputs(selectedTrigger ? {
        actionMode: selectedTrigger.action.mode ?? 'static',
        flowName: selectedTrigger.action.flow_name,
        projectPath: selectedTrigger.project_path,
    } : null).inputs
    const selectedTriggerForm = useMemo(
        () => (selectedTrigger ? triggerToFormState(selectedTrigger, savedFlowInputs ?? []) : null),
        [selectedTrigger, savedFlowInputs],
    )
    const newTriggerForm = newTriggerDraft.form
    const currentEditDraft = selectedTrigger ? editTriggerDraftsByTriggerId[triggerKey(selectedTrigger)] ?? null : null
    const resolvedEditTriggerForm = currentEditDraft?.form ?? selectedTriggerForm
    const dirty = Boolean(currentEditDraft?.form)
    const { inputs: newFlowInputs, retry: retryNewFlowInputs } = useFlowInputs(newTriggerForm)
    const { inputs: editFlowInputs, retry: retryEditFlowInputs } = useFlowInputs(resolvedEditTriggerForm)
    // Field errors show once a save is refused ('new' or the trigger's key), then follow the edits.
    const [errorsShownFor, setErrorsShownFor] = useState<string | null>(null)
    const newTriggerErrors = errorsShownFor === 'new' ? validateTriggerForm(newTriggerForm, false, newFlowInputs ?? []) : {}
    const editTriggerErrors = selectedTrigger && resolvedEditTriggerForm && errorsShownFor === triggerKey(selectedTrigger)
        ? validateTriggerForm(resolvedEditTriggerForm, selectedTrigger.protected, editFlowInputs ?? [])
        : {}
    const externalChange = dirty && currentEditDraft?.expectedRevision !== selectedTrigger?.revision
    useSettingsNavigationProtection(dirty, pending)

    // The trigger last saved, which says "Saved." until it's edited again or another is shown.
    const [savedFor, setSavedFor] = useState<string | null>(null)

    /** Resolves true once the trigger is created and selected. */
    const onCreateTrigger = async () => {
        // Until the flow's inputs are known, its required ones can't be checked.
        if (pendingRef.current || newFlowInputs === null) return false
        if (Object.keys(validateTriggerForm(newTriggerForm, false, newFlowInputs)).length) {
            setErrorsShownFor('new')
            return false
        }
        pendingRef.current = true
        setPending(true)
        try {
            const created = await createTriggerValidated({
                project_path: newTriggerForm.projectPath,
                name: newTriggerForm.name,
                enabled: newTriggerForm.enabled,
                source_type: newTriggerForm.sourceType,
                action: buildTriggerActionPayload(newTriggerForm, newFlowInputs ?? []),
                source: buildTriggerSourcePayload(newTriggerForm),
            })
            if (created.webhook_secret) {
                revealWebhookSecret(triggerKey(created), created.webhook_secret)
            }
            setTriggersSessionNewDraft({
                form: createEmptyTriggerForm(newTriggerForm.projectPath),
            })
            setErrorsShownFor(null)
            await refreshTriggers()
            setSelectedTriggerId(triggerKey(created))
            updateTriggersSession({ createFormOpen: false })
            return true
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to create trigger.')
            return false
        } finally {
            pendingRef.current = false
            setPending(false)
        }
    }

    const onSaveSelectedTrigger = async () => {
        if (pendingRef.current || !selectedTrigger || !resolvedEditTriggerForm || editFlowInputs === null) {
            return
        }
        if (Object.keys(validateTriggerForm(resolvedEditTriggerForm, selectedTrigger.protected, editFlowInputs ?? [])).length) {
            setErrorsShownFor(triggerKey(selectedTrigger))
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
                    action: { ...buildTriggerActionPayload(resolvedEditTriggerForm, editFlowInputs ?? []), mode: resolvedEditTriggerForm.actionMode },
                    source: buildTriggerSourcePayload(resolvedEditTriggerForm),
                }
            const updated = await updateTriggerValidated(selectedTrigger.project_path, selectedTrigger.id, { ...payload, expected_revision: currentEditDraft?.form ? currentEditDraft.expectedRevision ?? '' : selectedTrigger.revision })
            if (updated.webhook_secret) {
                revealWebhookSecret(triggerKey(selectedTrigger), updated.webhook_secret)
            }
            setTriggersSessionEditDraft(triggerKey(selectedTrigger), null)
            setErrorsShownFor(null)
            setSavedFor(triggerKey(selectedTrigger))
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
        saved: !dirty && selectedTrigger !== null && savedFor === triggerKey(selectedTrigger),
        externalChange,
        discard: () => {
            if (pendingRef.current || !selectedTrigger) return
            setTriggersSessionEditDraft(triggerKey(selectedTrigger), null)
            setErrorsShownFor(null)
            void refreshTriggers()
        },
        editTriggerForm: resolvedEditTriggerForm,
        editTriggerErrors,
        editFlowInputs,
        retryEditFlowInputs,
        newTriggerForm,
        newTriggerErrors,
        newFlowInputs,
        retryNewFlowInputs,
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
