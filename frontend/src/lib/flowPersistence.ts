import { useStore, type SaveErrorKind } from '@/store'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { fetchPreviewValidated, saveFlowValidated, type FlowSaveContent } from '@/lib/attractorClient'
import { recordFlowLoadDebug } from '@/lib/flowLoadDebug'
import { duplicateDecisionsInFlow } from '@/lib/canonicalFlowModel'
import { flowProblemsFromDetail } from '@/features/editor/model/flowLoadFailure'

interface SaveFlowErrorDetail {
    status?: string
    error?: string
}

const FALLBACK_SAVE_FAILURE_MESSAGE = 'Flow save failed before confirmation from backend.'
let lastSaveRequest: { name: string; content: FlowSaveContent; projectPath: string | null; environmentId: string | null } | null = null
const persistedBaselineByScope = new Map<string, string>()

// Flows belong to a project in an environment; the same name elsewhere is a different flow.
const activeProjectPath = () => useStore.getState().activeProjectPath

function getSaveScopeKey(name: string, projectPath: string | null, environmentId: string | null): string {
    return `${environmentId ?? 'local'}\n${projectPath ?? ''}\n${name}`
}

function parseErrorDetail(payload: unknown): SaveFlowErrorDetail {
    if (!payload || typeof payload !== 'object') {
        return {}
    }
    if ('detail' in payload && payload.detail && typeof payload.detail === 'object') {
        return payload.detail as SaveFlowErrorDetail
    }
    // The server's plain errors are `{"detail": "why"}`.
    if ('detail' in payload && typeof payload.detail === 'string' && payload.detail.trim()) {
        return { error: payload.detail.trim() }
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
        return error ?? 'The flow was changed elsewhere since it was opened.'
    }
    if (error) {
        return error
    }
    return `Flow save failed with HTTP ${statusCode}.`
}

export async function retryLastSaveContent(): Promise<boolean> {
    if (!lastSaveRequest) return false
    return saveFlowContent(lastSaveRequest.name, lastSaveRequest.content, lastSaveRequest.projectPath, lastSaveRequest.environmentId)
}

export function primeFlowSaveBaseline(
    name: string,
    content: FlowSaveContent,
    projectPath = activeProjectPath(),
    environmentId = getActiveEnvironmentId(),
): void {
    persistedBaselineByScope.set(getSaveScopeKey(name, projectPath, environmentId), JSON.stringify(content))
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

// Debounced saves not sent yet, one sender per scheduler.
const pendingSaveSenders = new Set<() => void>()

/** Registers a scheduler's sender for its debounced save; returns the unregister. */
export function registerPendingFlowSave(send: () => void): () => void {
    pendingSaveSenders.add(send)
    return () => { pendingSaveSenders.delete(send) }
}

/** Sends every debounced save now; true while any save is still in flight. */
export function sendPendingFlowSaves(): boolean {
    pendingSaveSenders.forEach((send) => send())
    return inFlightSaves.size > 0
}

// Editor fields whose text cannot be applied yet; saving waits for them.
const invalidDrafts = new Map<string, string>()

/** Records (or with `null`, clears) an editor draft that must block saving. */
export function setInvalidDraft(id: string, message: string | null) {
    if (message) {
        invalidDrafts.set(id, message)
    } else {
        invalidDrafts.delete(id)
    }
}

// The last save sent for each flow. A flow's saves go one at a time, in order: two requests in
// flight together could land out of order and leave older content on disk.
const lastSaveByScope = new Map<string, Promise<boolean>>()

/** Saves to the environment the save started in, by default the active one, even after a switch. */
export function saveFlowContent(
    name: string,
    content: FlowSaveContent,
    projectPath = activeProjectPath(),
    environmentId = getActiveEnvironmentId(),
): Promise<boolean> {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    const previous = lastSaveByScope.get(scopeKey)
    const save = previous
        ? previous.catch(() => false).then(() => persistFlowContent(name, content, projectPath, environmentId))
        : persistFlowContent(name, content, projectPath, environmentId)
    lastSaveByScope.set(scopeKey, save)
    inFlightSaves.add(save)
    void save.finally(() => {
        inFlightSaves.delete(save)
        if (lastSaveByScope.get(scopeKey) === save) lastSaveByScope.delete(scopeKey)
    })
    return save
}

async function persistFlowContent(
    name: string,
    content: FlowSaveContent,
    projectPath: string | null,
    environmentId: string | null,
): Promise<boolean> {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    const serialized = JSON.stringify(content)
    // The save state shown is the active environment's; a save that outlives a switch leaves it alone.
    const shown = () => (getActiveEnvironmentId() === environmentId ? useStore.getState() : null)
    if (persistedBaselineByScope.get(scopeKey) === serialized) {
        recordFlowLoadDebug('save:skipped', name, {
            contentLength: serialized.length,
            reason: 'baseline_match',
        })
        // The edits a save refused are undone: nothing is left unsaved.
        const state = shown()
        if (state?.saveState === 'error' || state?.saveState === 'conflict') state.resetSaveState()
        return true
    }
    const draftErrors = [...invalidDrafts.values()]
    if (draftErrors.length > 0) {
        shown()?.markSaveFailure(`Fix the editor's invalid input before saving: ${draftErrors.join(' ')}`, 'validation_error')
        return false
    }
    const duplicates = duplicateDecisionsInFlow('flow' in content ? content.flow : null)
    if (duplicates.length > 0) {
        shown()?.markSaveFailure(
            `Each decision may be declared once per node; duplicated: ${[...new Set(duplicates)].join(', ')}.`,
            'validation_error',
        )
        return false
    }
    // Retry repeats the save whose failure is shown.
    if (shown()) lastSaveRequest = { name, content, projectPath, environmentId }
    recordFlowLoadDebug('save:request', name, {
        contentLength: serialized.length,
    })
    shown()?.markSaveInFlight()

    try {
        // The server refuses a flow that doesn't validate, as one mid-edit often is (a step just added,
        // a route not yet wired); its preview answers either way, so the edits wait without a refused
        // save. A preview that fails says nothing: the save still goes, and the server decides.
        const preview = 'flow' in content
            ? await fetchPreviewValidated(content.flow, undefined, { flowName: name, projectPath, environmentId }).catch(() => null)
            : null
        if (preview?.status === 'validation_error') {
            const problems = flowProblemsFromDetail(preview)
            shown()?.markSaveFailure(buildErrorMessage('validation_error', problems?.map((problem) => problem.message).join(' ') ?? preview.error, 422), 'validation_error', problems)
            return false
        }
        const response = await saveFlowValidated(name, content, projectPath, environmentId)
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
                shown()?.markSaveConflict(message)
                return false
            }
            let errorKind: SaveErrorKind = 'http'
            if (detail.status === 'parse_error') {
                errorKind = 'parse_error'
            } else if (detail.status === 'validation_error') {
                errorKind = 'validation_error'
            }
            shown()?.markSaveFailure(message, errorKind, flowProblemsFromDetail(detail))
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
                shown()?.markSaveConflict(buildErrorMessage(status, undefined, response.statusCode))
                return false
            }
            shown()?.markSaveFailure(FALLBACK_SAVE_FAILURE_MESSAGE, 'unknown')
            return false
        }

        recordFlowLoadDebug('save:response', name, {
            ok: true,
            statusCode: response.statusCode,
            saveStatus: status,
        })
        persistedBaselineByScope.set(scopeKey, serialized)
        shown()?.markSaveSuccess()
        return true
    } catch (error) {
        const message = error instanceof Error ? error.message : 'network error while saving flow'
        recordFlowLoadDebug('save:error', name, {
            message,
        })
        shown()?.markSaveFailure(`Flow save failed: ${message}`, 'network')
        return false
    }
}
