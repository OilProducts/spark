import { useEffect, useMemo } from 'react'
import { fetchTriggerListValidated } from '@/lib/workspaceClient'
import { useStore } from '@/store'

export function useTriggersList({ manageSync = true }: { manageSync?: boolean } = {}) {
    const viewMode = useStore((state) => state.viewMode)
    const triggersSession = useStore((state) => state.triggersSession)
    const updateTriggersSession = useStore((state) => state.updateTriggersSession)
    const hasDraftedTriggerSession =
        triggersSession.scopeFilter !== 'all'
        || triggersSession.selectedTriggerId !== null
        || triggersSession.newTriggerDraft.form.name.trim().length > 0
        || Object.keys(triggersSession.editTriggerDraftsByTriggerId).length > 0
    const hasTriggersSession =
        viewMode === 'triggers'
        || triggersSession.status !== 'idle'
        || triggersSession.triggers.length > 0
        || hasDraftedTriggerSession

    const selectedTrigger = useMemo(
        () => triggersSession.triggers.find((trigger) => trigger.id === triggersSession.selectedTriggerId) ?? null,
        [triggersSession.selectedTriggerId, triggersSession.triggers],
    )
    const systemTriggers = useMemo(
        () => triggersSession.triggers.filter((trigger) => trigger.protected),
        [triggersSession.triggers],
    )
    const customTriggers = useMemo(
        () => triggersSession.triggers.filter((trigger) => !trigger.protected),
        [triggersSession.triggers],
    )

    const refreshTriggers = async () => {
        updateTriggersSession({
            status: 'loading',
        })
        try {
            const payload = await fetchTriggerListValidated()
            const currentSelectedTriggerId = useStore.getState().triggersSession.selectedTriggerId
            const nextTriggerIds = new Set(payload.map((trigger) => trigger.id))
            const currentSession = useStore.getState().triggersSession
            updateTriggersSession({
                triggers: payload,
                selectedTriggerId: currentSelectedTriggerId && nextTriggerIds.has(currentSelectedTriggerId)
                    ? currentSelectedTriggerId
                    : payload[0]?.id ?? null,
                revealedWebhookSecrets: Object.fromEntries(
                    Object.entries(currentSession.revealedWebhookSecrets).filter(([triggerId]) => nextTriggerIds.has(triggerId)),
                ),
                editTriggerDraftsByTriggerId: Object.fromEntries(
                    Object.entries(currentSession.editTriggerDraftsByTriggerId).filter(([triggerId]) => nextTriggerIds.has(triggerId)),
                ),
                error: null,
                status: 'ready',
            })
        } catch (nextError) {
            updateTriggersSession({
                error: nextError instanceof Error ? nextError.message : 'Unable to load triggers.',
                status: 'error',
            })
        }
    }

    useEffect(() => {
        if (!manageSync || !hasTriggersSession || triggersSession.status !== 'idle') {
            return
        }
        void refreshTriggers()
    }, [hasTriggersSession, manageSync, triggersSession.status])

    useEffect(() => {
        if (!manageSync || !hasTriggersSession) {
            return
        }
        const handleTriggerLiveEvent = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            const payload = detail?.payload
            if (detail?.type === 'trigger.snapshot' && Array.isArray(payload?.triggers)) {
                updateTriggersSession({
                    triggers: payload.triggers,
                    error: null,
                    status: 'ready',
                })
                return
            }
            if (detail?.type === 'trigger.upsert' && payload?.trigger) {
                const trigger = payload.trigger
                const current = useStore.getState().triggersSession.triggers
                const existingIndex = current.findIndex((entry) => entry.id === trigger.id)
                const nextTriggers = existingIndex === -1
                    ? [...current, trigger]
                    : current.map((entry, index) => index === existingIndex ? trigger : entry)
                updateTriggersSession({
                    triggers: nextTriggers,
                    error: null,
                    status: 'ready',
                })
                return
            }
            if (detail?.type === 'trigger.delete' && payload?.trigger?.id) {
                const triggerId = payload.trigger.id
                const nextTriggers = useStore.getState().triggersSession.triggers.filter((entry) => entry.id !== triggerId)
                updateTriggersSession({
                    triggers: nextTriggers,
                    selectedTriggerId: useStore.getState().triggersSession.selectedTriggerId === triggerId
                        ? nextTriggers[0]?.id ?? null
                        : useStore.getState().triggersSession.selectedTriggerId,
                    error: null,
                    status: 'ready',
                })
            }
        }
        window.addEventListener('spark:trigger-live-event', handleTriggerLiveEvent)
        window.addEventListener('spark:triggers-resync-required', refreshTriggers)
        return () => {
            window.removeEventListener('spark:trigger-live-event', handleTriggerLiveEvent)
            window.removeEventListener('spark:triggers-resync-required', refreshTriggers)
        }
    }, [hasTriggersSession, manageSync, updateTriggersSession])

    return {
        customTriggers,
        error: triggersSession.error,
        loading: triggersSession.status === 'loading',
        refreshTriggers,
        selectedTrigger,
        selectedTriggerId: triggersSession.selectedTriggerId,
        setError: (value: string | null) => updateTriggersSession({ error: value }),
        setSelectedTriggerId: (value: string | null) => updateTriggersSession({ selectedTriggerId: value }),
        status: triggersSession.status,
        systemTriggers,
        triggers: triggersSession.triggers,
    }
}
