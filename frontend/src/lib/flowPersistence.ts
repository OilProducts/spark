import { useStore, type SaveErrorKind } from '@/store'
import type { AppState } from '@/state/store-types'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { fetchPreviewValidated, saveFlowValidated, type FlowDefinitionPayload, type FlowSaveContent } from '@/lib/attractorClient'
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
    unsavedByScope.clear()
}

// The structured edits each flow's last save refused for a reason Retry can get past (the server
// unreachable, failing or changed), by scope, with how the refusal shows. They outlive the editor that
// made them, as a switch of environment or an interrupted load unmounts it: the flow opens with them,
// unsaved, until a save of it lands or they are discarded. Edits refused as invalid aren't kept: the
// flow may not open with them, so it opens as saved.
// ponytail: kept for this page's life; a reload drops them, as it drops any unsaved edit. A file view
// save refused is not kept; keep its files as drafts if that matters.
const unsavedByScope = new Map<string, { content: { flow: FlowDefinitionPayload }; show: (state: AppState) => void }>()

/** The edits a save of this flow refused, if any, showing that refusal with Retry. */
export function unsavedFlowEdits(
    name: string,
    projectPath = activeProjectPath(),
    environmentId = getActiveEnvironmentId(),
): FlowDefinitionPayload | null {
    const unsaved = unsavedByScope.get(getSaveScopeKey(name, projectPath, environmentId))
    if (!unsaved) return null
    lastSaveRequest = { name, content: unsaved.content, projectPath, environmentId }
    unsaved.show(useStore.getState())
    return unsaved.content.flow
}

/** Drops the edits a save of this flow refused, as discarding its unsaved edits or deleting it does. */
export function discardUnsavedFlowEdits(
    name: string,
    projectPath = activeProjectPath(),
    environmentId = getActiveEnvironmentId(),
): void {
    unsavedByScope.delete(getSaveScopeKey(name, projectPath, environmentId))
}

// Saves in flight, with the environment each goes to.
const inFlightSaves = new Map<Promise<boolean>, string | null>()

/** Waits for the active environment's saves in flight to settle; another environment's never hold it up. */
export async function waitForFlowSaves(): Promise<void> {
    const environmentId = getActiveEnvironmentId()
    await Promise.all([...inFlightSaves].filter(([, id]) => id === environmentId).map(([save]) => save))
}

// Debounced saves not sent yet, one sender per scheduler.
const pendingSaveSenders = new Set<() => void>()

/** Registers a scheduler's sender for its debounced save; returns the unregister. */
export function registerPendingFlowSave(send: () => void): () => void {
    pendingSaveSenders.add(send)
    return () => { pendingSaveSenders.delete(send) }
}

/** Sends every debounced save now; true while a save is still in flight, to `environmentId` when given. */
export function sendPendingFlowSaves(environmentId?: string | null): boolean {
    pendingSaveSenders.forEach((send) => send())
    return [...inFlightSaves.values()].some((id) => environmentId === undefined || id === environmentId)
}

// Editor fields whose text cannot be applied yet; saving in their environment waits for them.
const invalidDrafts = new Map<string, { message: string; environmentId: string | null }>()

/** Records (or with `null`, clears) an editor draft that must block saving in the active environment. */
export function setInvalidDraft(id: string, message: string | null) {
    if (message) {
        invalidDrafts.set(id, { message, environmentId: getActiveEnvironmentId() })
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
    inFlightSaves.set(save, environmentId)
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
    // Retry repeats the save whose failure is shown; structured edits it can save are kept for the flow.
    const fail = (show: (state: AppState) => void, retryable = true) => {
        if (retryable && 'flow' in content) unsavedByScope.set(scopeKey, { content, show })
        const state = shown()
        if (state) {
            lastSaveRequest = { name, content, projectPath, environmentId }
            show(state)
        }
        return false
    }
    if (persistedBaselineByScope.get(scopeKey) === serialized) {
        unsavedByScope.delete(scopeKey)
        recordFlowLoadDebug('save:skipped', name, {
            contentLength: serialized.length,
            reason: 'baseline_match',
        })
        // The edits a save refused are undone: nothing is left unsaved.
        const state = shown()
        if (state?.saveState === 'error' || state?.saveState === 'conflict') state.resetSaveState()
        return true
    }
    const draftErrors = [...invalidDrafts.values()].filter((draft) => draft.environmentId === environmentId).map((draft) => draft.message)
    if (draftErrors.length > 0) {
        return fail((state) => state.markSaveFailure(`Fix the editor's invalid input before saving: ${draftErrors.join(' ')}`, 'validation_error'), false)
    }
    const duplicates = duplicateDecisionsInFlow('flow' in content ? content.flow : null)
    if (duplicates.length > 0) {
        return fail((state) => state.markSaveFailure(
            `Each decision may be declared once per node; duplicated: ${[...new Set(duplicates)].join(', ')}.`,
            'validation_error',
        ), false)
    }
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
            return fail((state) => state.markSaveFailure(buildErrorMessage('validation_error', problems?.map((problem) => problem.message).join(' ') ?? preview.error, 422), 'validation_error', problems), false)
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
                return fail((state) => state.markSaveConflict(message))
            }
            let errorKind: SaveErrorKind = 'http'
            if (detail.status === 'parse_error') {
                errorKind = 'parse_error'
            } else if (detail.status === 'validation_error') {
                errorKind = 'validation_error'
            }
            return fail((state) => state.markSaveFailure(message, errorKind, flowProblemsFromDetail(detail)), errorKind === 'http')
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
                return fail((state) => state.markSaveConflict(buildErrorMessage(status, undefined, response.statusCode)))
            }
            return fail((state) => state.markSaveFailure(FALLBACK_SAVE_FAILURE_MESSAGE, 'unknown'))
        }

        recordFlowLoadDebug('save:response', name, {
            ok: true,
            statusCode: response.statusCode,
            saveStatus: status,
        })
        persistedBaselineByScope.set(scopeKey, serialized)
        unsavedByScope.delete(scopeKey)
        shown()?.markSaveSuccess()
        return true
    } catch (error) {
        const message = error instanceof Error ? error.message : 'network error while saving flow'
        recordFlowLoadDebug('save:error', name, {
            message,
        })
        return fail((state) => state.markSaveFailure(`Flow save failed: ${message}`, 'network'))
    }
}
