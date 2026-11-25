import { useStore, type SaveErrorKind } from '@/store'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { fetchFlowPayloadValidated, fetchPreviewValidated, saveFlowValidated, type FlowFile, type FlowSaveContent } from '@/lib/attractorClient'
import { fetchWorkspaceFlowFilesValidated } from '@/lib/api/flowsApi'
import { recordFlowLoadDebug } from '@/lib/flowLoadDebug'
import { duplicateDecisionsInFlow } from '@/lib/canonicalFlowModel'
import { flowProblemsFromDetail, type FlowProblem } from '@/features/editor/model/flowLoadFailure'
import { resolveSaveRemediation } from '@/lib/saveRemediation'

interface SaveFlowErrorDetail {
    status?: string
    error?: string
}

const FALLBACK_SAVE_FAILURE_MESSAGE = 'Flow save failed before confirmation from backend.'
let lastSaveRequest: { name: string; content: FlowSaveContent; projectPath: string | null; environmentId: string | null; opened: FlowOpening | undefined } | null = null
const persistedBaselineByScope = new Map<string, string>()
// One opening of a flow in the editor, from its load until the editor leaves it or loads it anew. A save
// belongs to the opening it was made in: only that one shows its state, and a later one never gets it.
// `files` are the flow's files as it loaded or a save from it verifiably left them, null when unknown;
// `edited` once edits are made in it; `restored` while it holds a refused save's edits, opened again over
// those files and not saved yet.
type FlowOpening = { files: string | null; closed: boolean; edited: boolean; restored?: boolean }
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

export async function retryLastSaveContent(): Promise<boolean> {
    const request = lastSaveRequest
    // Never over the flow as it opened anew.
    if (!request || openedByScope.get(getSaveScopeKey(request.name, request.projectPath, request.environmentId)) !== request.opened) return false
    return saveFlowContent(request.name, request.content, request.projectPath, request.environmentId)
}

/** Starts an opening of the flow, loaded as `content` from `files`; a flow its files don't make has no `content`. */
export function primeFlowSaveBaseline(
    name: string,
    content: FlowSaveContent | null,
    files: FlowFile[] | null = null,
    projectPath = activeProjectPath(),
    environmentId = getActiveEnvironmentId(),
): void {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    if (content) persistedBaselineByScope.set(scopeKey, JSON.stringify(content))
    else persistedBaselineByScope.delete(scopeKey)
    openedByScope.set(scopeKey, { files: files && JSON.stringify(files), closed: false, edited: false })
    if (refusedSaves.has(scopeKey)) refusedSavesChanged()
}

/** Ends the flow's opening as the editor leaves it: a save made in it is no longer shown there. */
export function closeFlowOpening(name: string, projectPath: string | null, environmentId: string | null): void {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    const opening = openedByScope.get(scopeKey)
    if (!opening) return
    opening.closed = true
    if (refusedSaves.has(scopeKey)) refusedSavesChanged()
}

/** Marks the flow's opening edited, as by drafts not saved yet: a save refused before it is no longer recovered over it. */
export function markFlowOpeningEdited(name: string, projectPath: string | null, environmentId: string | null): void {
    const scopeKey = getSaveScopeKey(name, projectPath, environmentId)
    const opening = openedByScope.get(scopeKey)
    if (!opening || opening.edited) return
    opening.edited = true
    if (refusedSaves.has(scopeKey)) refusedSavesChanged()
}

/** Marks the flow's opening as holding a refused save's edits, opened again: its saves first check the flow still has its files. */
export function markFlowOpeningRestored(name: string, projectPath: string | null, environmentId: string | null): void {
    const opening = openedByScope.get(getSaveScopeKey(name, projectPath, environmentId))
    if (opening) opening.restored = true
    markFlowOpeningEdited(name, projectPath, environmentId)
}

export function resetFlowSaveBaselines(): void {
    persistedBaselineByScope.clear()
    openedByScope.clear()
    lastSaveRequest = null
    refusedSaves.clear()
    refusedSavesChanged()
}

// Saves refused out of sight (another flow, environment or view shown, or the flow opened anew since), by
// scope, with the files their edits were made over. Their environment shows each until it is dismissed
// or recovered: once the flow has opened with those files and no edits since, its edits open again in the
// editor, to save at once or, refused as invalid, to correct; a flow saved since keeps its newer version.
// ponytail: kept for this page's life. Saves carry no revision, so a save elsewhere between recovery's last
// read of the files and its save still goes unseen, as does one between a save and the read of the files it
// left that changes only text the flow doesn't parse; a revision checked by the server would close both.
export type RefusedFlowSave = {
    name: string
    projectPath: string | null
    environmentId: string | null
    content: FlowSaveContent
    message: string
    problems: FlowProblem[] | null
    retryable: boolean
    baseFiles: string | null
    opened: FlowOpening | undefined
    // Refused as invalid, the canvas couldn't show them: the notice shows them to copy instead.
    unshowable?: boolean
}
const refusedSaves = new Map<string, RefusedFlowSave>()
const refusedSaveListeners = new Set<() => void>()
let refusedSavesSnapshot: RefusedFlowSave[] = []
function refusedSavesChanged() {
    refusedSavesSnapshot = [...refusedSaves.values()]
    refusedSaveListeners.forEach((listener) => listener())
}

/**
 * Dispatched with a refused save whose edits the editor showing its flow opens again, to save at once or,
 * refused as invalid, to correct; the editor dismisses it once they are open.
 */
export const FLOW_SAVE_RESTORED_EVENT = 'spark:flow-save-restored'

export function subscribeRefusedFlowSaves(listener: () => void): () => void {
    refusedSaveListeners.add(listener)
    return () => { refusedSaveListeners.delete(listener) }
}

export function refusedFlowSaves(): RefusedFlowSave[] {
    return refusedSavesSnapshot
}

/**
 * `open` while the editor that made the edits still has them, behind another view; `hidden` once it left
 * the flow, until the flow is open anew; then whether it opened with the edits' files and is unedited since.
 */
export function refusedFlowSaveStatus(refused: RefusedFlowSave): 'open' | 'hidden' | 'unchanged' | 'changed' {
    const current = openedByScope.get(getSaveScopeKey(refused.name, refused.projectPath, refused.environmentId))
    if (!current || current.closed) return 'hidden'
    if (current === refused.opened) return 'open'
    return !current.edited && refused.baseFiles !== null && current.files === refused.baseFiles ? 'unchanged' : 'changed'
}

const recoverable = (refused: RefusedFlowSave) =>
    refusedSaves.get(getSaveScopeKey(refused.name, refused.projectPath, refused.environmentId)) === refused
    && refusedFlowSaveStatus(refused) === 'unchanged'

function updateRefusedSave(refused: RefusedFlowSave, changes: Partial<RefusedFlowSave>): void {
    const scopeKey = getSaveScopeKey(refused.name, refused.projectPath, refused.environmentId)
    if (refusedSaves.get(scopeKey) !== refused) return
    refusedSaves.set(scopeKey, { ...refused, ...changes })
    refusedSavesChanged()
}

// The flow's files as saved now, whether or not they make a flow.
const savedFiles = (name: string, projectPath: string | null, environmentId: string | null) =>
    fetchWorkspaceFlowFilesValidated(name, projectPath, undefined, environmentId).then((files) => JSON.stringify(files))
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error))

// Reads the flow's files again, last thing before recovering its refused edits: a save landed since, from
// here or elsewhere, keeps its newer version, and the notice says so.
async function unchangedSince(refused: RefusedFlowSave): Promise<boolean> {
    let files: string
    try {
        files = await savedFiles(refused.name, refused.projectPath, refused.environmentId)
    } catch (error) {
        updateRefusedSave(refused, { message: `Couldn’t read the saved flow to recover these edits: ${errorText(error)}` })
        return false
    }
    if (files !== refused.baseFiles) {
        updateRefusedSave(refused, { baseFiles: null })
        return false
    }
    return recoverable(refused)
}

/**
 * Recovers a refused save while its flow, open anew, is unchanged since: the editor showing it opens its
 * edits again, so edits made there from then on are made over them. Edits waiting to save go first, as the
 * newer ones.
 */
export function retryRefusedFlowSave(refused: RefusedFlowSave): Promise<boolean> {
    const { name, projectPath, environmentId } = refused
    sendPendingFlowSaves()
    return queueSave(getSaveScopeKey(name, projectPath, environmentId), environmentId, async () => {
        if (!recoverable(refused) || !(await unchangedSince(refused))) return false
        window.dispatchEvent(new CustomEvent(FLOW_SAVE_RESTORED_EVENT, { detail: refused }))
        return true
    })
}

/** Keeps refused flow edits the canvas couldn't show for their notice, which shows them to copy instead. */
export function markRefusedFlowSaveUnshowable(refused: RefusedFlowSave): void {
    updateRefusedSave(refused, { unshowable: true })
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
    return queueSave(scopeKey, environmentId, () => persistFlowContent(name, content, projectPath, environmentId, opened))
}

function queueSave(scopeKey: string, environmentId: string | null, run: () => Promise<boolean>): Promise<boolean> {
    const previous = lastSaveByScope.get(scopeKey)
    const save = previous ? previous.catch(() => false).then(run) : run()
    lastSaveByScope.set(scopeKey, save)
    inFlightSaves.set(save, environmentId)
    void save.finally(() => {
        inFlightSaves.delete(save)
        if (lastSaveByScope.get(scopeKey) === save) lastSaveByScope.delete(scopeKey)
    })
    return save
}

// Each step's routes in order: previews sort them away, but a gate offers its decisions in it.
const routeOrder = (flow: Record<string, unknown>) => {
    const nodes = (flow.nodes ?? {}) as Record<string, { routes?: Record<string, unknown> } | null>
    return JSON.stringify(Object.keys(nodes).sort().map((id) => [id, ...Object.keys(nodes[id]?.routes ?? {})]))
}

// The files a structured save left, read back. They count only if their flow previews as the one the save
// sent, with its routes in their order: a save made elsewhere meanwhile is never taken for this one's, and
// leaves them unknown.
// ponytail: two reads per structured save; a revision answered by the save would replace them.
async function filesLeftBy(sent: Record<string, unknown>, sentPreview: Record<string, unknown>, name: string, projectPath: string | null, environmentId: string | null): Promise<string | null> {
    try {
        const { files, flow } = await fetchFlowPayloadValidated(name, projectPath, undefined, environmentId)
        const read = await fetchPreviewValidated(flow, undefined, { flowName: name, projectPath, environmentId })
        return JSON.stringify(read.flow) === JSON.stringify(sentPreview) && routeOrder(flow) === routeOrder(sent) ? JSON.stringify(files) : null
    } catch {
        return null
    }
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
    // The save state shown is this flow's, in the opening of it that made the save: never another flow's
    // or environment's, nor this flow's once left or opened anew.
    const shown = () => {
        const state = useStore.getState()
        return getActiveEnvironmentId() === environmentId && state.activeFlow === name && state.activeProjectPath === projectPath
            && openedByScope.get(scopeKey) === opened && !opened?.closed ? state : null
    }
    // Retry repeats the save whose failure is shown; one refused out of sight, even behind another view
    // of its editor, is also kept for its environment's notices.
    const fail = (message: string, kind: SaveErrorKind, problems: FlowProblem[] | null = null) => {
        const state = shown()
        const retryable = resolveSaveRemediation('error', kind)?.allowRetry ?? false
        if (state) {
            lastSaveRequest = { name, content, projectPath, environmentId, opened }
            if (kind === 'conflict') state.markSaveConflict(message)
            else state.markSaveFailure(message, kind, problems)
        }
        if (state?.viewMode !== 'editor') {
            refusedSaves.set(scopeKey, { name, projectPath, environmentId, content, message, problems, retryable, baseFiles: opened?.files ?? null, opened })
            refusedSavesChanged()
        }
        return false
    }
    // Made before the flow opened anew, these edits would replace what it shows unseen: its notice keeps them.
    if (openedByScope.get(scopeKey) !== opened) {
        return fail('These edits weren’t sent: the flow opened again first.', 'unknown')
    }
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
    // Edits made in the flow's opening are newer than any refused before it opened.
    markFlowOpeningEdited(name, projectPath, environmentId)
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
        // Restored edits save only over the files they were made on, read last thing before: once the flow
        // was saved elsewhere, no save from this opening goes.
        if (opened?.restored) {
            let files: string
            try {
                files = await savedFiles(name, projectPath, environmentId)
            } catch (error) {
                return fail(`Couldn’t read the saved flow to save these edits over it: ${errorText(error)}`, 'network')
            }
            if (files !== opened.files) {
                return fail('The flow was saved elsewhere since these edits were made, so they weren’t saved over it.', 'conflict')
            }
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
        // Edits made from here on are over the files this save left, read back before a later save lands. A
        // files save is read by the editor's reload after it, which opens the flow anew.
        if (opened) {
            opened.restored = false
            opened.files = 'flow' in content && preview?.flow ? await filesLeftBy(content.flow, preview.flow, name, projectPath, environmentId) : null
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
