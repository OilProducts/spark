import { useStore, type SaveErrorKind } from '@/store'
import { saveFlowValidated, type FlowSaveContent } from '@/lib/attractorClient'
import { recordFlowLoadDebug } from '@/lib/flowLoadDebug'

interface SaveFlowErrorDetail {
    status?: string
    error?: string
}

const FALLBACK_SAVE_FAILURE_MESSAGE = 'Flow save failed before confirmation from backend.'
let lastSaveRequest: { name: string; content: FlowSaveContent; projectPath: string | null } | null = null
const persistedBaselineByScope = new Map<string, string>()

// Flows belong to a project; the same name in another project is a different flow.
const activeProjectPath = () => useStore.getState().activeProjectPath

function getSaveScopeKey(name: string, projectPath: string | null): string {
    return `${projectPath ?? ''}\n${name}`
}

function parseErrorDetail(payload: unknown): SaveFlowErrorDetail {
    if (!payload || typeof payload !== 'object') {
        return {}
    }
    if ('detail' in payload && payload.detail && typeof payload.detail === 'object') {
        return payload.detail as SaveFlowErrorDetail
    }
    return payload as SaveFlowErrorDetail
}

function buildErrorMessage(status: string | undefined, error: string | undefined, statusCode: number): string {
    if (status === 'parse_error') {
        return `Save blocked by a flow parse error: ${error ?? FALLBACK_SAVE_FAILURE_MESSAGE}`
    }
    if (status === 'validation_error') {
        return `Save blocked by validation errors: ${error ?? FALLBACK_SAVE_FAILURE_MESSAGE}`
    }
    if (status === 'conflict' || statusCode === 409) {
        return `Save conflict detected: ${error ?? 'The flow was modified elsewhere. Refresh and re-apply your changes.'}`
    }
    if (error) {
        return error
    }
    return `Flow save failed with HTTP ${statusCode}.`
}

export async function retryLastSaveContent(): Promise<boolean> {
    if (!lastSaveRequest) return false
    return saveFlowContent(lastSaveRequest.name, lastSaveRequest.content, lastSaveRequest.projectPath)
}

export function primeFlowSaveBaseline(name: string, content: FlowSaveContent, projectPath = activeProjectPath()): void {
    persistedBaselineByScope.set(getSaveScopeKey(name, projectPath), JSON.stringify(content))
}

export function resetFlowSaveBaselines(): void {
    persistedBaselineByScope.clear()
    lastSaveRequest = null
}

const inFlightSaves = new Set<Promise<boolean>>()

/** Waits for every save in flight to settle. */
export async function waitForFlowSaves(): Promise<void> {
    await Promise.all([...inFlightSaves])
}

export function saveFlowContent(name: string, content: FlowSaveContent, projectPath = activeProjectPath()): Promise<boolean> {
    const save = persistFlowContent(name, content, projectPath)
    inFlightSaves.add(save)
    void save.finally(() => inFlightSaves.delete(save))
    return save
}

async function persistFlowContent(name: string, content: FlowSaveContent, projectPath: string | null): Promise<boolean> {
    const scopeKey = getSaveScopeKey(name, projectPath)
    const serialized = JSON.stringify(content)
    if (persistedBaselineByScope.get(scopeKey) === serialized) {
        recordFlowLoadDebug('save:skipped', name, {
            contentLength: serialized.length,
            reason: 'baseline_match',
        })
        return true
    }
    const { markSaveInFlight, markSaveSuccess, markSaveFailure, markSaveConflict } = useStore.getState()
    lastSaveRequest = { name, content, projectPath }
    recordFlowLoadDebug('save:request', name, {
        contentLength: serialized.length,
    })
    markSaveInFlight()

    try {
        const response = await saveFlowValidated(name, content, projectPath)
        if (!response.ok) {
            const detail = parseErrorDetail(response.payload)
            const message = buildErrorMessage(detail.status, detail.error, response.statusCode)
            recordFlowLoadDebug('save:response', name, {
                ok: false,
                statusCode: response.statusCode,
                saveStatus: detail.status ?? null,
                message,
            })
            if (detail.status === 'conflict' || response.statusCode === 409) {
                markSaveConflict(message)
                return false
            }
            let errorKind: SaveErrorKind = 'http'
            if (detail.status === 'parse_error') {
                errorKind = 'parse_error'
            } else if (detail.status === 'validation_error') {
                errorKind = 'validation_error'
            }
            markSaveFailure(message, errorKind)
            return false
        }

        const status = typeof (response.payload as { status?: unknown } | null)?.status === 'string'
            ? (response.payload as { status: string }).status
            : undefined
        if (status !== 'saved') {
            recordFlowLoadDebug('save:response', name, {
                ok: false,
                statusCode: response.statusCode,
                saveStatus: status ?? null,
                message: FALLBACK_SAVE_FAILURE_MESSAGE,
            })
            if (status === 'conflict') {
                markSaveConflict(buildErrorMessage(status, undefined, response.statusCode))
                return false
            }
            markSaveFailure(FALLBACK_SAVE_FAILURE_MESSAGE, 'unknown')
            return false
        }

        recordFlowLoadDebug('save:response', name, {
            ok: true,
            statusCode: response.statusCode,
            saveStatus: status,
        })
        persistedBaselineByScope.set(scopeKey, serialized)
        markSaveSuccess()
        return true
    } catch (error) {
        const message = error instanceof Error ? error.message : 'network error while saving flow'
        recordFlowLoadDebug('save:error', name, {
            message,
        })
        markSaveFailure(`Flow save failed: ${message}`, 'network')
        return false
    }
}
