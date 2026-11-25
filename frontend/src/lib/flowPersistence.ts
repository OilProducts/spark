import { useStore, type SaveErrorKind } from '@/store'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { fetchPreviewValidated, saveFlowValidated, type FlowSaveContent } from '@/lib/attractorClient'
import { recordFlowLoadDebug } from '@/lib/flowLoadDebug'
import { duplicateDecisionsInFlow } from '@/lib/canonicalFlowModel'
import { flowProblemsFromDetail, type FlowProblem } from '@/features/editor/model/flowLoadFailure'
import { resolveSaveRemediation } from '@/lib/saveRemediation'

interface SaveFlowErrorDetail {
    status?: string
    error?: string
}

const FALLBACK_SAVE_FAILURE_MESSAGE = 'Flow save failed before confirmation from backend.'
let lastSaveRequest: { name: string; content: FlowSaveContent; projectPath: string | null; environmentId: string | null } | null = null
const persistedBaselineByScope = new Map<string, string>()
// Which opening of each flow (its baseline primed from the server) is the latest.
const openedByScope = new Map<string, number>()
let openings = 0

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
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    persistedBaselineByScope.set(scopeKey, JSON.stringify(content))
    openedByScope.set(scopeKey, ++openings)
    if (refusedSaves.has(scopeKey)) refusedSavesChanged()
}

export function resetFlowSaveBaselines(): void {
    persistedBaselineByScope.clear()
    openedByScope.clear()
    lastSaveRequest = null
    refusedSaves.clear()
    refusedSavesChanged()
}

// Saves refused while their flow wasn't shown (another flow or environment open, or the flow opened
// anew since), by scope, with the content saved before their edits. Their environment shows each until
// it is dismissed or saved: once the flow has opened still as those edits found it, Retry may send them;
// once it is saved since, nothing does, so the newer version stays.
// ponytail: kept for this page's life; flow saves carry no revision, so Retry still replaces a version
// saved elsewhere after the flow opened. Send the base revision if that matters.
export type RefusedFlowSave = {
    name: string
    projectPath: string | null
    environmentId: string | null
    content: FlowSaveContent
    message: string
    retryable: boolean
    expectedBaseline: string | null
    opened: number | undefined
}
const refusedSaves = new Map<string, RefusedFlowSave>()
const refusedSaveListeners = new Set<() => void>()
let refusedSavesSnapshot: RefusedFlowSave[] = []
function refusedSavesChanged() {
    refusedSavesSnapshot = [...refusedSaves.values()]
    refusedSaveListeners.forEach((listener) => listener())
}

/** Dispatched once a refused save's edits are saved, so an editor showing the flow as it was reloads it. */
export const FLOW_SAVE_RECOVERED_EVENT = 'spark:flow-save-recovered'

export function subscribeRefusedFlowSaves(listener: () => void): () => void {
    refusedSaveListeners.add(listener)
    return () => { refusedSaveListeners.delete(listener) }
}

export function refusedFlowSaves(): RefusedFlowSave[] {
    return refusedSavesSnapshot
}

/** `hidden` until the flow opens again; then whether it is still as the refused edits found it. */
export function refusedFlowSaveStatus(refused: RefusedFlowSave): 'hidden' | 'unchanged' | 'changed' {
    const scopeKey = getSaveScopeKey(refused.name, refused.projectPath, refused.environmentId)
    if (openedByScope.get(scopeKey) === refused.opened) return 'hidden'
    return (persistedBaselineByScope.get(scopeKey) ?? null) === refused.expectedBaseline ? 'unchanged' : 'changed'
}

/** Sends a refused save's edits again, only while its flow, opened since, is still as they found it. */
export async function retryRefusedFlowSave(refused: RefusedFlowSave): Promise<boolean> {
    if (!refused.retryable || refusedFlowSaveStatus(refused) !== 'unchanged') return false
    return saveFlowContent(refused.name, refused.content, refused.projectPath, refused.environmentId)
}

export function dismissRefusedFlowSave(refused: RefusedFlowSave): void {
    const scopeKey = getSaveScopeKey(refused.name, refused.projectPath, refused.environmentId)
    if (refusedSaves.get(scopeKey) !== refused) return
    refusedSaves.delete(scopeKey)
    refusedSavesChanged()
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
    const expectedBaseline = persistedBaselineByScope.get(scopeKey) ?? null
    const opened = openedByScope.get(scopeKey)
    // The save state shown is this flow's, in the editor that sent the save: never another flow's or
    // environment's, nor this flow's once opened anew, as it then shows what is saved, not these edits.
    const shown = () => {
        const state = useStore.getState()
        return getActiveEnvironmentId() === environmentId && state.activeFlow === name && state.activeProjectPath === projectPath
            && openedByScope.get(scopeKey) === opened ? state : null
    }
    // Retry repeats the save whose failure is shown; one refused out of sight is kept for its environment.
    const fail = (message: string, kind: SaveErrorKind, problems: FlowProblem[] | null = null) => {
        const state = shown()
        if (state) {
            lastSaveRequest = { name, content, projectPath, environmentId }
            if (kind === 'conflict') state.markSaveConflict(message)
            else state.markSaveFailure(message, kind, problems)
        } else {
            const retryable = resolveSaveRemediation('error', kind)?.allowRetry ?? false
            refusedSaves.set(scopeKey, { name, projectPath, environmentId, content, message, retryable, expectedBaseline, opened })
            refusedSavesChanged()
        }
        return false
    }
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
    const draftErrors = [...invalidDrafts.values()].filter((draft) => draft.environmentId === environmentId).map((draft) => draft.message)
    if (draftErrors.length > 0) {
        return fail(`Fix the editor's invalid input before saving: ${draftErrors.join(' ')}`, 'validation_error')
    }
    const duplicates = duplicateDecisionsInFlow('flow' in content ? content.flow : null)
    if (duplicates.length > 0) {
        return fail(`Each decision may be declared once per node; duplicated: ${[...new Set(duplicates)].join(', ')}.`, 'validation_error')
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
            return fail(buildErrorMessage('validation_error', problems?.map((problem) => problem.message).join(' ') ?? preview.error, 422), 'validation_error', problems)
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
                return fail(message, 'conflict')
            }
            let errorKind: SaveErrorKind = 'http'
            if (detail.status === 'parse_error') {
                errorKind = 'parse_error'
            } else if (detail.status === 'validation_error') {
                errorKind = 'validation_error'
            }
            return fail(message, errorKind, flowProblemsFromDetail(detail))
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
                return fail(buildErrorMessage(status, undefined, response.statusCode), 'conflict')
            }
            return fail(FALLBACK_SAVE_FAILURE_MESSAGE, 'unknown')
        }

        recordFlowLoadDebug('save:response', name, {
            ok: true,
            statusCode: response.statusCode,
            saveStatus: status,
        })
        persistedBaselineByScope.set(scopeKey, serialized)
        shown()?.markSaveSuccess()
        const refused = refusedSaves.get(scopeKey)
        if (refused) {
            // Its edits are saved: sent again, or carried by a later save of the flow before it opened anew.
            const recovered = JSON.stringify(refused.content) === serialized
            if (recovered || openedByScope.get(scopeKey) === refused.opened) refusedSaves.delete(scopeKey)
            refusedSavesChanged()
            if (recovered) window.dispatchEvent(new CustomEvent(FLOW_SAVE_RECOVERED_EVENT, { detail: { name, projectPath, environmentId } }))
        }
        return true
    } catch (error) {
        const message = error instanceof Error ? error.message : 'network error while saving flow'
        recordFlowLoadDebug('save:error', name, {
            message,
        })
        return fail(`Flow save failed: ${message}`, 'network')
    }
}
