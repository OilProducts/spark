import { useCallback, useEffect, useRef } from 'react'
import { saveFlowContent } from '@/lib/flowPersistence'
import { useStore } from '@/store'
import type { FlowSaveContent } from '@/lib/attractorClient'

interface PendingFlowSaveRequest {
    flowName: string
    content: FlowSaveContent
    projectPath: string | null
}

interface UseFlowSaveSchedulerArgs<T> {
    flowName: string | null
    debounceMs: number
    buildContent: (payload: T | undefined, flowName: string) => FlowSaveContent
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
        // Files mode is the only writer. The switch into it saves the current
        // structured state, so a structured save that lands later (one queued
        // before the switch, or one from canvas changes) is stale and would
        // overwrite file text.
        if (useStore.getState().editorMode === 'files') {
            return Promise.resolve(false)
        }
        return saveFlowContent(request.flowName, request.content, request.projectPath)
    }, [])

    const createSaveRequest = useCallback((payload: T | undefined): PendingFlowSaveRequest | null => {
        if (!flowName) {
            return null
        }
        return {
            flowName,
            content: buildContent(payload, flowName),
            // Captured now so a debounced save cannot land in a project switched to meanwhile.
            projectPath: useStore.getState().activeProjectPath,
        }
    }, [buildContent, flowName])

    const flushPendingSave = useCallback(() => {
        const pendingSave = pendingSaveRef.current
        if (!pendingSave) {
            return
        }
        clearScheduledSave()
        pendingSaveRef.current = null
        void executeSaveRequest(pendingSave)
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
            void executeSaveRequest(pendingSave)
        }, debounceMs)
    }, [clearScheduledSave, createSaveRequest, debounceMs, executeSaveRequest])

    /** Saves the given content now; resolves true once it is persisted. */
    const saveNow = useCallback((payload?: T): Promise<boolean> => {
        const request = createSaveRequest(payload)
        if (!request) {
            return Promise.resolve(false)
        }
        clearPendingSave()
        return executeSaveRequest(request)
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
