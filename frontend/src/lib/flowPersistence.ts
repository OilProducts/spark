import { useStore, type SaveErrorKind } from '@/store'
import { saveFlowValidated } from '@/lib/attractorClient'
import { recordFlowLoadDebug } from '@/lib/flowLoadDebug'

interface SaveFlowErrorDetail {
    status?: string
    error?: string
}

export interface SaveFlowOptions {
    expectSemanticEquivalence?: boolean
}

const FALLBACK_SAVE_FAILURE_MESSAGE = 'Flow save failed before confirmation from backend.'
export const EXPECT_SEMANTIC_EQUIVALENCE_OPTIONS: SaveFlowOptions = { expectSemanticEquivalence: true }
let lastSaveRequest: { name: string; content: string; options?: SaveFlowOptions } | null = null
const persistedBaselineByScope = new Map<string, string>()

function getSaveScopeKey(name: string): string {
    return name
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
        return `Save blocked by DOT parse error: ${error ?? FALLBACK_SAVE_FAILURE_MESSAGE}`
    }
    if (status === 'validation_error') {
        return `Save blocked by validation errors: ${error ?? FALLBACK_SAVE_FAILURE_MESSAGE}`
    }
    if (status === 'semantic_mismatch') {
        return `Save blocked by semantic equivalence check: ${error ?? 'A no-op save would change flow behavior.'}`
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
    return saveFlowContent(lastSaveRequest.name, lastSaveRequest.content, lastSaveRequest.options)
}

export function primeFlowSaveBaseline(name: string, content: string): void {
    persistedBaselineByScope.set(getSaveScopeKey(name), content)
}

export function resetFlowSaveBaselines(): void {
    persistedBaselineByScope.clear()
    lastSaveRequest = null
}

export async function saveFlowContentExpectingSemanticEquivalence(name: string, content: string): Promise<boolean> {
    return saveFlowContent(name, content, EXPECT_SEMANTIC_EQUIVALENCE_OPTIONS)
}

export async function saveFlowContent(name: string, content: string, options?: SaveFlowOptions): Promise<boolean> {
    const scopeKey = getSaveScopeKey(name)
    const existingBaseline = persistedBaselineByScope.get(scopeKey)
    if (existingBaseline === content) {
        recordFlowLoadDebug('save:skipped', name, {
            contentLength: content.length,
            expectSemanticEquivalence: options?.expectSemanticEquivalence === true,
            reason: 'baseline_match',
        })
        return true
    }
    const { markSaveInFlight, markSaveSuccess, markSaveFailure, markSaveConflict } = useStore.getState()
    lastSaveRequest = { name, content, options }
    recordFlowLoadDebug('save:request', name, {
        contentLength: content.length,
        expectSemanticEquivalence: options?.expectSemanticEquivalence === true,
    })
    markSaveInFlight()

    try {
        const response = await saveFlowValidated(name, content, options?.expectSemanticEquivalence === true)
        if (!response.ok) {
            const detail = parseErrorDetail(response.payload)
            const message = buildErrorMessage(detail.status, detail.error, response.statusCode)
            recordFlowLoadDebug('save:response', name, {
                ok: false,
                statusCode: response.statusCode,
                saveStatus: detail.status ?? null,
                message,
                expectSemanticEquivalence: options?.expectSemanticEquivalence === true,
            })
            if (detail.status === 'conflict' || (response.statusCode === 409 && detail.status !== 'semantic_mismatch')) {
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
                expectSemanticEquivalence: options?.expectSemanticEquivalence === true,
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
            expectSemanticEquivalence: options?.expectSemanticEquivalence === true,
        })
        persistedBaselineByScope.set(scopeKey, content)
        markSaveSuccess()
        return true
    } catch (error) {
        const message = error instanceof Error ? error.message : 'network error while saving flow'
        recordFlowLoadDebug('save:error', name, {
            message,
            expectSemanticEquivalence: options?.expectSemanticEquivalence === true,
        })
        markSaveFailure(`Flow save failed: ${message}`, 'network')
        return false
    }
}
