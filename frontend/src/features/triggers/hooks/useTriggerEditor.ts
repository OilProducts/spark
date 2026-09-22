import { useEffect, useMemo, useRef, useState } from 'react'
import {
    createTriggerValidated,
    deleteTriggerValidated,
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
type UseTriggerEditorArgs = {
    activeProjectPath: string | null
    refreshTriggers: () => Promise<void>
    selectedTrigger: TriggerResponse | null
    setError: (value: string | null) => void
    revealWebhookSecret: (triggerId: string, secret: string) => void
    setSelectedTriggerId: (value: string | null) => void
}

const resolveActiveTargetFields = (activeProjectPath: string | null) => ({
    targetMode: activeProjectPath ? 'active' as const : 'none' as const,
    projectPath: activeProjectPath ?? '',
})

const applyActiveTargetFields = (
    form: TriggerFormState,
    activeProjectPath: string | null,
): TriggerFormState => {
    const nextTargetFields = resolveActiveTargetFields(activeProjectPath)
    if (
        form.targetMode === nextTargetFields.targetMode
        && form.projectPath === nextTargetFields.projectPath
    ) {
        return form
    }
    return {
        ...form,
        ...nextTargetFields,
    }
}

const applyInferredTargetFields = (
    form: TriggerFormState,
    selectedTrigger: TriggerResponse,
    activeProjectPath: string | null,
): TriggerFormState => {
    const inferredTarget = triggerToFormState(selectedTrigger, activeProjectPath)
    if (
        form.targetMode === inferredTarget.targetMode
        && form.projectPath === inferredTarget.projectPath
    ) {
        return form
    }
    return {
        ...form,
        targetMode: inferredTarget.targetMode,
        projectPath: inferredTarget.projectPath,
    }
}

const didTriggerTargetChange = (current: TriggerFormState, next: TriggerFormState) => (
    current.targetMode !== next.targetMode
    || current.projectPath !== next.projectPath
)

const buildProtectedTriggerUpdatePayload = (form: TriggerFormState) => ({
    name: form.name,
    enabled: form.enabled,
    action: {
        flow_name: form.flowName.trim(),
    },
})

export function useTriggerEditor({
    activeProjectPath,
    refreshTriggers,
    selectedTrigger,
    setError,
    revealWebhookSecret,
    setSelectedTriggerId,
}: UseTriggerEditorArgs) {
    const { confirm } = useDialogController()
    const pendingRef = useRef(false)
    const [pending, setPending] = useState(false)
    const activeProjectPathRef = useRef(activeProjectPath)
    const newTriggerDraft = useStore((state) => state.triggersSession.newTriggerDraft)
    const editTriggerDraftsByTriggerId = useStore((state) => state.triggersSession.editTriggerDraftsByTriggerId)
    const setTriggersSessionNewDraft = useStore((state) => state.setTriggersSessionNewDraft)
    const setTriggersSessionEditDraft = useStore((state) => state.setTriggersSessionEditDraft)
    const updateTriggersSession = useStore((state) => state.updateTriggersSession)

    const selectedTriggerForm = useMemo(
        () => (selectedTrigger ? triggerToFormState(selectedTrigger, activeProjectPath) : null),
        [activeProjectPath, selectedTrigger],
    )
    const newTriggerForm = newTriggerDraft.form
    const currentEditDraft = selectedTrigger ? editTriggerDraftsByTriggerId[selectedTrigger.id] ?? null : null
    const resolvedEditTriggerForm = currentEditDraft?.form ?? selectedTriggerForm
    const dirty = Boolean(currentEditDraft?.form)
    const externalChange = dirty && currentEditDraft?.expectedRevision !== selectedTrigger?.revision
    useSettingsNavigationProtection(dirty, pending)

    useEffect(() => {
        activeProjectPathRef.current = activeProjectPath
    }, [activeProjectPath])

    useEffect(() => {
        if (newTriggerDraft.targetBehavior === 'manual') {
            return
        }
        const syncedForm = applyActiveTargetFields(newTriggerDraft.form, activeProjectPath)
        if (syncedForm === newTriggerDraft.form) {
            return
        }
        setTriggersSessionNewDraft({
            ...newTriggerDraft,
            form: syncedForm,
        })
    }, [activeProjectPath, newTriggerDraft, setTriggersSessionNewDraft])

    useEffect(() => {
        if (!selectedTrigger || !currentEditDraft?.form
            || (currentEditDraft.expectedRevision && currentEditDraft.expectedRevision !== selectedTrigger.revision)) {
            return
        }
        if (currentEditDraft.targetBehavior === 'manual') {
            return
        }
        const syncedForm = currentEditDraft.targetBehavior === 'active'
            ? applyActiveTargetFields(currentEditDraft.form, activeProjectPath)
            : applyInferredTargetFields(currentEditDraft.form, selectedTrigger, activeProjectPath)
        if (syncedForm === currentEditDraft.form) {
            return
        }
        setTriggersSessionEditDraft(selectedTrigger.id, {
            ...currentEditDraft,
            form: syncedForm,
        })
    }, [activeProjectPath, currentEditDraft, selectedTrigger, setTriggersSessionEditDraft])

    const onCreateTrigger = async () => {
        if (pendingRef.current) return
        pendingRef.current = true
        setPending(true)
        try {
            const created = await createTriggerValidated({
                name: newTriggerForm.name,
                enabled: newTriggerForm.enabled,
                source_type: newTriggerForm.sourceType,
                action: buildTriggerActionPayload(newTriggerForm),
                source: buildTriggerSourcePayload(newTriggerForm),
            })
            if (created.webhook_secret) {
                revealWebhookSecret(created.id, created.webhook_secret)
            }
            setTriggersSessionNewDraft({
                form: createEmptyTriggerForm(activeProjectPathRef.current),
                targetBehavior: 'default',
            })
            await refreshTriggers()
            setSelectedTriggerId(created.id)
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
                    action: buildTriggerActionPayload(resolvedEditTriggerForm),
                    source: buildTriggerSourcePayload(resolvedEditTriggerForm),
                }
            const updated = await updateTriggerValidated(selectedTrigger.id, { ...payload, expected_revision: currentEditDraft?.form ? currentEditDraft.expectedRevision ?? '' : selectedTrigger.revision })
            if (updated.webhook_secret) {
                revealWebhookSecret(updated.id, updated.webhook_secret)
            }
            setTriggersSessionEditDraft(selectedTrigger.id, null)
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
            await deleteTriggerValidated(selectedTrigger.id, currentEditDraft?.form ? currentEditDraft.expectedRevision ?? '' : selectedTrigger.revision)
            setTriggersSessionEditDraft(selectedTrigger.id, null)
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
            setTriggersSessionEditDraft(selectedTrigger.id, null)
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
            const currentForm = currentDraft?.form ?? selectedTriggerForm ?? next
            const targetBehavior = currentForm && next && didTriggerTargetChange(currentForm, next)
                ? (next.targetMode === 'active' ? 'active' : 'manual')
                : currentDraft?.targetBehavior ?? 'inferred'
            setTriggersSessionEditDraft(selectedTrigger.id, {
                triggerId: selectedTrigger.id,
                expectedRevision: currentDraft?.form ? currentDraft.expectedRevision ?? '' : selectedTrigger.revision,
                form: next,
                targetBehavior,
            })
        },
        setNewTriggerForm: (next: TriggerFormState) => {
            if (pendingRef.current) return
            setTriggersSessionNewDraft({
                form: next,
                targetBehavior: didTriggerTargetChange(newTriggerDraft.form, next)
                    ? (next.targetMode === 'active' ? 'active' : 'manual')
                    : newTriggerDraft.targetBehavior,
            })
        },
    }
}
