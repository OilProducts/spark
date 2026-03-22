import { useEffect, useMemo, useState } from 'react'
import { fetchTriggerListValidated, type TriggerResponse } from '@/lib/workspaceClient'

export function useTriggersList() {
    const [triggers, setTriggers] = useState<TriggerResponse[]>([])
    const [selectedTriggerId, setSelectedTriggerId] = useState<string | null>(null)
    const [error, setError] = useState<string | null>(null)
    const [loading, setLoading] = useState(false)

    const selectedTrigger = useMemo(
        () => triggers.find((trigger) => trigger.id === selectedTriggerId) ?? null,
        [selectedTriggerId, triggers],
    )
    const systemTriggers = useMemo(
        () => triggers.filter((trigger) => trigger.protected),
        [triggers],
    )
    const customTriggers = useMemo(
        () => triggers.filter((trigger) => !trigger.protected),
        [triggers],
    )

    const refreshTriggers = async () => {
        setLoading(true)
        try {
            const payload = await fetchTriggerListValidated()
            setTriggers(payload)
            setSelectedTriggerId((current) => current ?? payload[0]?.id ?? null)
            setError(null)
        } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : 'Unable to load triggers.')
        } finally {
            setLoading(false)
        }
    }

    useEffect(() => {
        void refreshTriggers()
    }, [])

    return {
        customTriggers,
        error,
        loading,
        refreshTriggers,
        selectedTrigger,
        selectedTriggerId,
        setError,
        setSelectedTriggerId,
        systemTriggers,
        triggers,
    }
}
