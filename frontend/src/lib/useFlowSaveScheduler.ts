import { useCallback, useEffect, useRef } from 'react'
import { saveFlowContent } from '@/lib/flowPersistence'

interface PendingFlowSaveRequest {
    flowName: string
    content: string
}

interface UseFlowSaveSchedulerArgs<T> {
    flowName: string | null
    debounceMs: number
    buildContent: (payload: T | undefined, flowName: string) => string
}

export function useFlowSaveScheduler<T = void>({
    flowName,
    debounceMs,
    buildContent,
}: UseFlowSaveSchedulerArgs<T>) {
    const saveTimerRef = useRef<number | null>(null)
    const pendingSaveRef = useRef<PendingFlowSaveRequest | null>(null)

    const clearScheduledSave = useCallback(() => {
        if (saveTimerRef.current !== null) {
            window.clearTimeout(saveTimerRef.current)
            saveTimerRef.current = null
        }
    }, [])

    const executeSaveRequest = useCallback((request: PendingFlowSaveRequest) => {
        void saveFlowContent(request.flowName, request.content)
    }, [])

    const createSaveRequest = useCallback((payload: T | undefined): PendingFlowSaveRequest | null => {
        if (!flowName) {
            return null
        }
        return {
            flowName,
            content: buildContent(payload, flowName),
        }
    }, [buildContent, flowName])

    const flushPendingSave = useCallback(() => {
        const pendingSave = pendingSaveRef.current
        if (!pendingSave) {
            return
        }
        clearScheduledSave()
        pendingSaveRef.current = null
        executeSaveRequest(pendingSave)
    }, [clearScheduledSave, executeSaveRequest])

    const clearPendingSave = useCallback(() => {
        pendingSaveRef.current = null
        clearScheduledSave()
    }, [clearScheduledSave])

    const scheduleSave = useCallback((payload?: T) => {
        const request = createSaveRequest(payload)
        if (!request) {
            return
        }
        pendingSaveRef.current = request
        clearScheduledSave()
        saveTimerRef.current = window.setTimeout(() => {
            const pendingSave = pendingSaveRef.current
            pendingSaveRef.current = null
            saveTimerRef.current = null
            if (!pendingSave) {
                return
            }
            executeSaveRequest(pendingSave)
        }, debounceMs)
    }, [clearScheduledSave, createSaveRequest, debounceMs, executeSaveRequest])

    const saveNow = useCallback((payload?: T) => {
        const request = createSaveRequest(payload)
        if (!request) {
            return
        }
        clearPendingSave()
        executeSaveRequest(request)
    }, [clearPendingSave, createSaveRequest, executeSaveRequest])

    useEffect(() => {
        const handleBeforeUnload = () => {
            flushPendingSave()
        }
        window.addEventListener('beforeunload', handleBeforeUnload)
        return () => {
            window.removeEventListener('beforeunload', handleBeforeUnload)
            flushPendingSave()
        }
    }, [flushPendingSave])

    return {
        clearPendingSave,
        flushPendingSave,
        saveNow,
        scheduleSave,
    }
}
