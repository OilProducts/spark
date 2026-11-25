import { useStore, type SaveErrorKind } from '@/store'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { fetchPreviewValidated, saveFlowValidated, type FlowSaveContent } from '@/lib/attractorClient'
import { recordFlowLoadDebug } from '@/lib/flowLoadDebug'
import { duplicateDecisionsInFlow } from '@/lib/canonicalFlowModel'
import { flowProblemsFromDetail, type FlowProblem } from '@/features/editor/model/flowLoadFailure'

interface SaveFlowErrorDetail {
    status?: string
    error?: string
}

const FALLBACK_SAVE_FAILURE_MESSAGE = 'Flow save failed before confirmation from backend.'
let lastSaveRequest: { name: string; content: FlowSaveContent; projectPath: string | null; environmentId: string | null; opened: FlowOpening | undefined } | null = null
const persistedBaselineByScope = new Map<string, string>()
// One opening of a flow in the editor, from its load until the editor leaves it or loads it anew. A save
// belongs to the opening it was made in: only that one shows its state, and a later one never gets it.
type FlowOpening = { closed: boolean }
const openedByScope = new Map<string, FlowOpening>()

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

/**
 * The store while it shows the flow in the opening a save was made in: never another flow's or
 * environment's, nor this flow's once left or opened anew.
 */
function shownOpening(name: string, projectPath: string | null, environmentId: string | null, opened: FlowOpening | undefined) {
    const state = useStore.getState()
    return getActiveEnvironmentId() === environmentId && state.activeFlow === name && state.activeProjectPath === projectPath
        && openedByScope.get(getSaveScopeKey(name, projectPath, environmentId)) === opened && !opened?.closed ? state : null
}

export async function retryLastSaveContent(): Promise<boolean> {
    const request = lastSaveRequest
    // Only while the flow shows in the opening that made the save: never over another flow, nor this one opened anew.
    if (!request || !shownOpening(request.name, request.projectPath, request.environmentId, request.opened)) return false
    return saveFlowContent(request.name, request.content, request.projectPath, request.environmentId)
}

/** Starts an opening of the flow, loaded as `content`; a flow its files don't make has no `content`. */
export function primeFlowSaveBaseline(
    name: string,
    content: FlowSaveContent | null,
    projectPath = activeProjectPath(),
    environmentId = getActiveEnvironmentId(),
): void {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    if (content) persistedBaselineByScope.set(scopeKey, JSON.stringify(content))
    else persistedBaselineByScope.delete(scopeKey)
    openedByScope.set(scopeKey, { closed: false })
    if (refusedSaves.has(scopeKey)) refusedSavesChanged()
}

/** Ends the flow's opening as the editor leaves it: a save made in it is no longer shown there. */
export function closeFlowOpening(name: string, projectPath: string | null, environmentId: string | null): void {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    const opening = openedByScope.get(scopeKey)
    if (!opening) return
    opening.closed = true
    if (lastSaveRequest?.opened === opening) lastSaveRequest = null
    if (refusedSaves.has(scopeKey)) refusedSavesChanged()
}

export function resetFlowSaveBaselines(): void {
    persistedBaselineByScope.clear()
    openedByScope.clear()
    lastSaveRequest = null
    refusedSaves.clear()
    refusedSavesChanged()
}

// Saves refused out of sight (another flow, environment or view shown, or the flow opened anew since), by
// scope. Their environment shows each, with its edits to review and make again by hand, until it is
// dismissed. Nothing sends them again, nor judges them against the flow as it reads now.
// ponytail: kept for this page's life. Saves carry no revision (SaveFlowRequest in attractor-api), so a
// recovered snapshot sent again could overwrite a version saved after any check; a revision checked by
// the server would let recovery save them.
export type RefusedFlowSave = {
    name: string
    projectPath: string | null
    environmentId: string | null
    content: FlowSaveContent
    message: string
    kind: SaveErrorKind
    problems: FlowProblem[] | null
    opened: FlowOpening | undefined
}
const refusedSaves = new Map<string, RefusedFlowSave>()
const refusedSaveListeners = new Set<() => void>()
let refusedSavesSnapshot: RefusedFlowSave[] = []
function refusedSavesChanged() {
    refusedSavesSnapshot = [...refusedSaves.values()]
    refusedSaveListeners.forEach((listener) => listener())
}

export function subscribeRefusedFlowSaves(listener: () => void): () => void {
    refusedSaveListeners.add(listener)
    return () => { refusedSaveListeners.delete(listener) }
}

export function refusedFlowSaves(): RefusedFlowSave[] {
    return refusedSavesSnapshot
}

/** `open` while the editor that made the edits still has them, behind another view; otherwise `hidden`. */
export function refusedFlowSaveStatus(refused: RefusedFlowSave): 'open' | 'hidden' {
    const current = openedByScope.get(getSaveScopeKey(refused.name, refused.projectPath, refused.environmentId))
    return current && !current.closed && current === refused.opened ? 'open' : 'hidden'
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
    const opened = openedByScope.get(scopeKey)
    const previous = lastSaveByScope.get(scopeKey)
    const save = previous
        ? previous.catch(() => false).then(() => persistFlowContent(name, content, projectPath, environmentId, opened))
        : persistFlowContent(name, content, projectPath, environmentId, opened)
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
    opened: FlowOpening | undefined,
): Promise<boolean> {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    const serialized = JSON.stringify(content)
    // The save state shown is this flow's, in the opening of it that made the save.
    const shown = () => shownOpening(name, projectPath, environmentId, opened)
    // Retry repeats the save whose failure is shown; one refused out of sight, even behind another view
    // of its editor, is also kept for its environment's notices.
    const fail = (message: string, kind: SaveErrorKind, problems: FlowProblem[] | null = null) => {
        const state = shown()
        if (state) {
            lastSaveRequest = { name, content, projectPath, environmentId, opened }
            if (kind === 'conflict') state.markSaveConflict(message)
            else state.markSaveFailure(message, kind, problems)
        }
        if (state?.viewMode !== 'editor') {
            refusedSaves.set(scopeKey, { name, projectPath, environmentId, content, message, kind, problems, opened })
            refusedSavesChanged()
        }
        return false
    }
    // Made before the flow opened anew, these edits would replace what it shows unseen: its notice keeps them.
    const openedAnew = () => openedByScope.get(scopeKey) !== opened
    const keepUnsent = () => fail('These edits weren’t sent: the flow opened again first.', 'unknown')
    if (openedAnew()) return keepUnsent()
    // A later save made in the same opening carries the edits of one refused before it.
    const refused = refusedSaves.get(scopeKey)
    if (refused && refused.opened === opened) {
        refusedSaves.delete(scopeKey)
        refusedSavesChanged()
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
        // The flow can open anew while the preview answers.
        if (openedAnew()) return keepUnsent()
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
        // An opening since then keeps the baseline it loaded: its flow may not read as this save left it.
        if (!openedAnew()) persistedBaselineByScope.set(scopeKey, serialized)
        shown()?.markSaveSuccess()
        return true
    } catch (error) {
        const message = error instanceof Error ? error.message : 'network error while saving flow'
        recordFlowLoadDebug('save:error', name, {
            message,
        })
        return fail(`Flow save failed: ${message}`, 'network')
    }
}
