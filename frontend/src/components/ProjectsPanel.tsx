import { type PlanStatus, type ProjectRegistrationResult, useStore } from "@/store"
import { type ChangeEvent, type FormEvent, type KeyboardEvent, type PointerEvent as ReactPointerEvent, useEffect, useMemo, useRef, useState } from "react"
import { ChevronDown, ChevronUp, FileText, Folder, FolderOpen, Plus, Trash2 } from "lucide-react"
import {
    ApiHttpError,
    type ConversationSummaryResponse,
    type ConversationSnapshotResponse,
    type ConversationTurnEventStreamResponse,
    type ConversationTurnUpsertEventResponse,
    type ConversationTurnEventResponse,
    type ConversationTurnResponse,
    deleteConversationValidated,
    type ExecutionCardResponse,
    type SpecEditProposalResponse,
    approveSpecEditProposalValidated,
    fetchConversationSnapshotValidated,
    fetchProjectRegistryValidated,
    fetchProjectConversationListValidated,
    pickProjectDirectoryValidated,
    parseConversationSnapshotResponse,
    parseConversationStreamEventResponse,
    rejectSpecEditProposalValidated,
    registerProjectValidated,
    reviewExecutionCardValidated,
    sendConversationTurnValidated,
    updateProjectStateValidated,
} from "@/lib/apiClient"
import { useNarrowViewport } from "@/lib/useNarrowViewport"
import { isAbsoluteProjectPath, normalizeProjectPath } from "@/lib/projectPaths"
import { HomeProjectSidebar } from "@/components/HomeProjectSidebar"
import { HomeWorkspace } from "@/components/HomeWorkspace"

const buildProjectConversationId = (projectPath: string) => {
    const normalizedProjectKey = projectPath
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/(^-|-$)/g, "")
    const suffix = normalizedProjectKey || "project"
    const randomSuffix = typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
        ? crypto.randomUUID().slice(0, 8)
        : Math.random().toString(36).slice(2, 10)
    return `conversation-${suffix}-${randomSuffix}`
}

const PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT = 12
const DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT = 320
const HOME_SIDEBAR_MIN_PRIMARY_HEIGHT = 208
const HOME_SIDEBAR_MIN_SECONDARY_HEIGHT = 208
const HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT = 12
const CONVERSATION_BOTTOM_THRESHOLD_PX = 24

type ProjectGitMetadata = {
    branch: string | null
    commit: string | null
}

type PickerFileWithPath = File & {
    path?: string
    webkitRelativePath?: string
}

type SurfaceTone = "neutral" | "info" | "success" | "warning" | "danger"

const EMPTY_PROJECT_GIT_METADATA: ProjectGitMetadata = {
    branch: null,
    commit: null,
}

const SURFACE_TONE_CLASS_MAP: Record<SurfaceTone, string> = {
    neutral: "bg-muted/50 text-muted-foreground",
    info: "bg-sky-500/15 text-sky-700",
    success: "bg-emerald-500/15 text-emerald-800",
    warning: "bg-amber-500/15 text-amber-800",
    danger: "bg-destructive/10 text-destructive",
}

const getSurfaceToneClassName = (tone: SurfaceTone) => (
    `rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${SURFACE_TONE_CLASS_MAP[tone]}`
)

const getSpecEditStatusPresentation = (status: SpecEditProposalResponse["status"]) => {
    if (status === "applied") {
        return { label: "Applied", tone: "success" as const }
    }
    if (status === "rejected") {
        return { label: "Rejected", tone: "danger" as const }
    }
    return { label: "Pending review", tone: "warning" as const }
}

const getExecutionCardStatusPresentation = (status: ExecutionCardResponse["status"]) => {
    if (status === "approved") {
        return { label: "Approved", tone: "success" as const }
    }
    if (status === "rejected") {
        return { label: "Rejected", tone: "danger" as const }
    }
    if (status === "revision-requested") {
        return { label: "Revision requested", tone: "warning" as const }
    }
    return { label: "Draft", tone: "info" as const }
}

const getToolCallStatusPresentation = (status: "running" | "completed" | "failed") => {
    if (status === "running") {
        return { label: "Running", tone: "info" as const }
    }
    if (status === "failed") {
        return { label: "Failed", tone: "danger" as const }
    }
    return { label: "Completed", tone: "success" as const }
}

const derivePlanStatusFromExecutionCard = (executionCard: ExecutionCardResponse | null): PlanStatus => {
    if (!executionCard) {
        return "draft"
    }
    return executionCard.status
}

const asProjectGitMetadataField = (value: unknown): string | null => {
    if (typeof value !== "string") {
        return null
    }
    const trimmed = value.trim()
    return trimmed.length > 0 ? trimmed : null
}

const parseAbsoluteProjectPath = (value: string): { prefix: string; segments: string[] } | null => {
    const normalized = normalizeProjectPath(value)
    if (!isAbsoluteProjectPath(normalized)) {
        return null
    }
    if (normalized.startsWith("/")) {
        return { prefix: "/", segments: normalized.slice(1).split("/").filter(Boolean) }
    }
    const windowsPrefixMatch = normalized.match(/^[A-Za-z]:\//)
    if (!windowsPrefixMatch) {
        return null
    }
    return {
        prefix: windowsPrefixMatch[0],
        segments: normalized.slice(windowsPrefixMatch[0].length).split("/").filter(Boolean),
    }
}

const buildAbsoluteProjectPath = (prefix: string, segments: string[]) => {
    if (segments.length === 0) {
        return prefix
    }
    return `${prefix}${segments.join("/")}`
}

const deriveCommonAbsoluteDirectory = (directoryPaths: string[]): string | null => {
    const parsedDirectories = directoryPaths
        .map((path) => parseAbsoluteProjectPath(path))
        .filter((parsed): parsed is { prefix: string; segments: string[] } => Boolean(parsed))
    if (parsedDirectories.length === 0) {
        return null
    }
    const firstPrefix = parsedDirectories[0].prefix
    if (parsedDirectories.some((parsed) => parsed.prefix !== firstPrefix)) {
        return null
    }
    let commonSegments = [...parsedDirectories[0].segments]
    for (const parsed of parsedDirectories.slice(1)) {
        let index = 0
        while (
            index < commonSegments.length
            && index < parsed.segments.length
            && commonSegments[index] === parsed.segments[index]
        ) {
            index += 1
        }
        commonSegments = commonSegments.slice(0, index)
    }
    if (commonSegments.length === 0) {
        return firstPrefix
    }
    return buildAbsoluteProjectPath(firstPrefix, commonSegments)
}

const deriveProjectPathFromDirectorySelection = (files: FileList | null): string | null => {
    if (!files || files.length === 0) {
        return null
    }
    const inferredProjectPaths: string[] = []
    const fallbackDirectories: string[] = []
    for (const file of Array.from(files)) {
        const pickerFile = file as PickerFileWithPath
        const rawAbsoluteFilePath = typeof pickerFile.path === "string" ? pickerFile.path : ""
        const absoluteFilePath = normalizeProjectPath(rawAbsoluteFilePath)
        if (!absoluteFilePath || !isAbsoluteProjectPath(absoluteFilePath)) {
            continue
        }
        const fileSlashIndex = absoluteFilePath.lastIndexOf("/")
        if (fileSlashIndex <= 0) {
            continue
        }
        const absoluteDirectoryPath = normalizeProjectPath(absoluteFilePath.slice(0, fileSlashIndex))
        if (absoluteDirectoryPath && isAbsoluteProjectPath(absoluteDirectoryPath)) {
            fallbackDirectories.push(absoluteDirectoryPath)
        }

        const rawRelativePath = typeof pickerFile.webkitRelativePath === "string"
            ? pickerFile.webkitRelativePath.trim()
            : ""
        if (!rawRelativePath) {
            continue
        }
        const relativePath = normalizeProjectPath(rawRelativePath).replace(/^\/+/, "")
        if (!relativePath || !absoluteFilePath.endsWith(relativePath)) {
            continue
        }
        const basePath = normalizeProjectPath(absoluteFilePath.slice(0, absoluteFilePath.length - relativePath.length))
        const relativeSegments = relativePath.split("/").filter(Boolean)
        if (!basePath || relativeSegments.length === 0) {
            continue
        }
        const inferredProjectPath = normalizeProjectPath(`${basePath}/${relativeSegments[0]}`)
        if (inferredProjectPath && isAbsoluteProjectPath(inferredProjectPath)) {
            inferredProjectPaths.push(inferredProjectPath)
        }
    }

    const uniqueInferredPaths = [...new Set(inferredProjectPaths)]
    if (uniqueInferredPaths.length > 0) {
        uniqueInferredPaths.sort((left, right) => left.length - right.length)
        return uniqueInferredPaths[0]
    }

    return deriveCommonAbsoluteDirectory(fallbackDirectories)
}

const formatProjectListLabel = (projectPath: string) => {
    const normalizedPath = normalizeProjectPath(projectPath)
    const segments = normalizedPath.split("/").filter(Boolean)
    if (segments.length === 0) {
        return normalizedPath
    }
    return segments[segments.length - 1]
}

const toHydratedProjectRecord = (project: {
    project_path: string
    is_favorite: boolean
    last_accessed_at?: string | null
    active_conversation_id?: string | null
}) => ({
    directoryPath: project.project_path,
    isFavorite: project.is_favorite === true,
    lastAccessedAt: typeof project.last_accessed_at === "string" ? project.last_accessed_at : null,
    activeConversationId: typeof project.active_conversation_id === "string" ? project.active_conversation_id : null,
})

const formatConversationAgeShort = (value: string) => {
    const parsed = new Date(value)
    if (Number.isNaN(parsed.getTime())) {
        return ""
    }
    const deltaMs = Date.now() - parsed.getTime()
    if (deltaMs <= 0) {
        return "now"
    }
    const minuteMs = 60_000
    const hourMs = 60 * minuteMs
    const dayMs = 24 * hourMs
    const weekMs = 7 * dayMs
    if (deltaMs < hourMs) {
        return `${Math.max(1, Math.round(deltaMs / minuteMs))}m`
    }
    if (deltaMs < dayMs) {
        return `${Math.max(1, Math.round(deltaMs / hourMs))}h`
    }
    if (deltaMs < weekMs) {
        return `${Math.max(1, Math.round(deltaMs / dayMs))}d`
    }
    return `${Math.max(1, Math.round(deltaMs / weekMs))}w`
}

const clampHomeSidebarPrimaryHeight = (height: number, containerHeight: number) => {
    if (containerHeight <= 0) {
        return Math.max(height, HOME_SIDEBAR_MIN_PRIMARY_HEIGHT)
    }
    const maxPrimaryHeight = Math.max(
        HOME_SIDEBAR_MIN_PRIMARY_HEIGHT,
        containerHeight - HOME_SIDEBAR_MIN_SECONDARY_HEIGHT - HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT,
    )
    return Math.min(Math.max(height, HOME_SIDEBAR_MIN_PRIMARY_HEIGHT), maxPrimaryHeight)
}

const extractApiErrorMessage = (error: unknown, fallback: string) => {
    if (error instanceof ApiHttpError && error.detail) {
        return error.detail
    }
    if (error instanceof Error && error.message) {
        return error.message
    }
    return fallback
}

const isProjectChatDebugEnabled = () => {
    if (typeof window === "undefined") {
        return false
    }
    try {
        const params = new URLSearchParams(window.location.search)
        if (params.get("debugProjectChat") === "1") {
            return true
        }
        return window.localStorage.getItem("sparkspawn.debug.project_chat") === "1"
    } catch {
        return false
    }
}

const summarizeConversationTurnsForDebug = (turns: ConversationSnapshotResponse["turns"]) => (
    turns.map((turn, index) => ({
        index,
        id: turn.id,
        role: turn.role,
        kind: turn.kind,
        status: turn.status,
        artifactId: turn.artifact_id ?? null,
        content: turn.content.slice(0, 120),
    }))
)

const debugProjectChat = (message: string, details?: Record<string, unknown>) => {
    if (!isProjectChatDebugEnabled()) {
        return
    }
    if (details) {
        console.debug(`[project-chat] ${message}`, details)
        return
    }
    console.debug(`[project-chat] ${message}`)
}

const getLatestApprovedSpecEditProposal = (snapshot: ConversationSnapshotResponse | null) => {
    if (!snapshot) {
        return null
    }
    for (let index = snapshot.spec_edit_proposals.length - 1; index >= 0; index -= 1) {
        const proposal = snapshot.spec_edit_proposals[index]
        if (proposal?.status === "applied") {
            return proposal
        }
    }
    return null
}

const getLatestExecutionCard = (snapshot: ConversationSnapshotResponse | null) => {
    if (!snapshot || snapshot.execution_cards.length === 0) {
        return null
    }
    return snapshot.execution_cards[snapshot.execution_cards.length - 1] || null
}

type ConversationTimelineEntry =
    | {
        id: string
        kind: "message"
        role: "user" | "assistant"
        content: string
        timestamp: string
        status: ConversationTurnResponse["status"]
        error?: string | null
        presentation?: "default" | "thinking"
    }
    | {
        id: string
        kind: "tool_call"
        role: "system"
        timestamp: string
        toolCall: {
            id: string
            kind: "command_execution" | "file_change" | "dynamic_tool"
            status: "running" | "completed" | "failed"
            title: string
            command?: string | null
            output?: string | null
            filePaths: string[]
        }
    }
    | {
        id: string
        kind: "spec_edit_proposal" | "execution_card"
        role: "system"
        artifactId: string
        timestamp: string
    }

type OptimisticSendState = {
    conversationId: string
    projectPath: string
    message: string
    createdAt: string
}

const ensureConversationSnapshotShell = (
    conversationId: string,
    projectPath: string,
    title = "New thread",
): ConversationSnapshotResponse => ({
    conversation_id: conversationId,
    project_path: projectPath,
    title,
    created_at: "",
    updated_at: "",
    turns: [],
    turn_events: [],
    event_log: [],
    spec_edit_proposals: [],
    execution_cards: [],
    execution_workflow: {
        status: "idle",
        run_id: null,
        error: null,
        flow_source: null,
    },
})

const upsertConversationTurn = (
    snapshot: ConversationSnapshotResponse,
    turn: ConversationTurnResponse,
): ConversationSnapshotResponse => {
    const nextTurns = [...snapshot.turns]
    const existingIndex = nextTurns.findIndex((entry) => entry.id === turn.id)
    if (existingIndex >= 0) {
        nextTurns[existingIndex] = turn
    } else {
        nextTurns.push(turn)
    }
    return {
        ...snapshot,
        turns: nextTurns,
    }
}

const appendConversationTurnEvent = (
    snapshot: ConversationSnapshotResponse,
    event: ConversationTurnEventResponse,
): ConversationSnapshotResponse => {
    if (snapshot.turn_events.some((entry) => entry.id === event.id)) {
        return snapshot
    }
    return {
        ...snapshot,
        turn_events: [...snapshot.turn_events, event],
    }
}

const isTransientConversationTurnEvent = (event: ConversationTurnEventResponse) => (
    event.kind === 'assistant_delta'
)

const sortConversationTurnEvents = (events: ConversationTurnEventResponse[]) => (
    [...events].sort((left, right) => {
        if (left.turn_id === right.turn_id) {
            const sequenceDelta = left.sequence - right.sequence
            if (sequenceDelta !== 0) {
                return sequenceDelta
            }
            const timestampDelta = left.timestamp.localeCompare(right.timestamp)
            if (timestampDelta !== 0) {
                return timestampDelta
            }
            return left.id.localeCompare(right.id)
        }
        return left.timestamp.localeCompare(right.timestamp)
    })
)

const appendLiveConversationTurnEvent = (
    events: ConversationTurnEventResponse[],
    event: ConversationTurnEventResponse,
): ConversationTurnEventResponse[] => {
    if (events.some((entry) => entry.id === event.id)) {
        return events
    }
    return sortConversationTurnEvents([...events, event])
}

const pruneLiveConversationTurnEvents = (
    events: ConversationTurnEventResponse[],
    snapshot: ConversationSnapshotResponse,
): ConversationTurnEventResponse[] => {
    const snapshotTurnById = new Map(snapshot.turns.map((turn) => [turn.id, turn]))
    return events.filter((event) => {
        if (!isTransientConversationTurnEvent(event)) {
            return false
        }
        return snapshotTurnById.has(event.turn_id)
    })
}

const sanitizeStreamingTurnUpsert = (
    currentTurn: ConversationTurnResponse | null,
    incomingTurn: ConversationTurnResponse,
): ConversationTurnResponse => {
    if (incomingTurn.role !== 'assistant') {
        return incomingTurn
    }
    if (incomingTurn.status !== 'pending' && incomingTurn.status !== 'streaming') {
        return incomingTurn
    }
    return {
        ...incomingTurn,
        content: currentTurn?.content ?? '',
    }
}

const buildAssistantTimelineEntries = (
    turn: ConversationTurnResponse,
    turnEvents: ConversationTurnEventResponse[],
): ConversationTimelineEntry[] => {
    const entries: ConversationTimelineEntry[] = []
    let currentAssistantMessageIndex = -1
    let currentThinkingIndex = -1
    let accumulatedAssistantContent = ""

    const startAssistantEntry = (
        timestamp: string,
        content: string,
        presentation: "default" | "thinking",
        status: ConversationTurnResponse["status"],
    ) => {
        const entry: ConversationTimelineEntry = {
            id: `${turn.id}:${presentation}:${entries.length}`,
            kind: "message",
            role: "assistant",
            content,
            timestamp,
            status,
            error: turn.error ?? null,
            presentation,
        }
        entries.push(entry)
        if (presentation === "thinking") {
            currentThinkingIndex = entries.length - 1
        } else {
            currentAssistantMessageIndex = entries.length - 1
        }
    }

    const appendAssistantMessageDelta = (timestamp: string, contentDelta: string) => {
        if (currentAssistantMessageIndex < 0) {
            startAssistantEntry(timestamp, contentDelta, "default", turn.status)
        } else {
            const entry = entries[currentAssistantMessageIndex]
            if (entry.kind === "message") {
                entry.content = `${entry.content}${contentDelta}`
                entry.status = turn.status
                entry.error = turn.error ?? null
            }
        }
        accumulatedAssistantContent = `${accumulatedAssistantContent}${contentDelta}`
    }

    const appendReasoningSummaryDelta = (timestamp: string, contentDelta: string) => {
        if (currentThinkingIndex < 0) {
            startAssistantEntry(timestamp, contentDelta, "thinking", "streaming")
        } else {
            const entry = entries[currentThinkingIndex]
            if (entry.kind === "message") {
                entry.content = `${entry.content}${contentDelta}`
                entry.status = "streaming"
            }
        }
    }

    const resetAssistantSegmentPointers = () => {
        currentAssistantMessageIndex = -1
        currentThinkingIndex = -1
    }

    turnEvents.forEach((event) => {
        if (event.kind === "assistant_delta") {
            appendAssistantMessageDelta(event.timestamp || turn.timestamp, event.content_delta || "")
            return
        }
        if (event.kind === "reasoning_summary") {
            appendReasoningSummaryDelta(event.timestamp || turn.timestamp, event.content_delta || "")
            return
        }
        if (event.tool_call && event.tool_call_id) {
            const nextEntry: ConversationTimelineEntry = {
                id: event.tool_call_id,
                kind: "tool_call",
                role: "system",
                timestamp: event.timestamp,
                toolCall: {
                    id: event.tool_call.id,
                    kind: event.tool_call.kind,
                    status: event.tool_call.status,
                    title: event.tool_call.title,
                    command: event.tool_call.command ?? null,
                    output: event.tool_call.output ?? null,
                    filePaths: event.tool_call.file_paths,
                },
            }
            const existingIndex = entries.findIndex((entry) => entry.kind === "tool_call" && entry.id === event.tool_call_id)
            if (existingIndex >= 0) {
                entries[existingIndex] = nextEntry
            } else {
                entries.push(nextEntry)
            }
            resetAssistantSegmentPointers()
            return
        }
        if (event.kind === "assistant_completed") {
            if (!accumulatedAssistantContent && turn.content.trim()) {
                startAssistantEntry(event.timestamp || turn.timestamp, turn.content, "default", "complete")
            } else if (turn.content.startsWith(accumulatedAssistantContent)) {
                const remainingContent = turn.content.slice(accumulatedAssistantContent.length)
                if (remainingContent) {
                    startAssistantEntry(event.timestamp || turn.timestamp, remainingContent, "default", "complete")
                    accumulatedAssistantContent = turn.content
                } else if (currentAssistantMessageIndex >= 0) {
                    const entry = entries[currentAssistantMessageIndex]
                    if (entry.kind === "message") {
                        entry.status = "complete"
                    }
                }
            } else if (currentAssistantMessageIndex >= 0) {
                const entry = entries[currentAssistantMessageIndex]
                if (entry.kind === "message") {
                    entry.content = turn.content
                    entry.status = "complete"
                }
            } else if (turn.content.trim()) {
                startAssistantEntry(event.timestamp || turn.timestamp, turn.content, "default", "complete")
            }
            currentThinkingIndex = -1
            return
        }
        if (event.kind === "assistant_failed") {
            if (currentAssistantMessageIndex >= 0) {
                const entry = entries[currentAssistantMessageIndex]
                if (entry.kind === "message") {
                    entry.status = "failed"
                    entry.error = turn.error ?? event.message ?? null
                }
            } else {
                startAssistantEntry(
                    event.timestamp || turn.timestamp,
                    turn.content || event.message || "",
                    "default",
                    "failed",
                )
            }
            currentThinkingIndex = -1
        }
    })

    if (entries.length === 0 && turn.content.trim() && (turn.status === "complete" || turn.status === "failed")) {
        startAssistantEntry(turn.timestamp, turn.content, "default", turn.status)
    }

    const needsTrailingThinkingEntry = (
        (turn.status === "pending" || turn.status === "streaming")
        && currentAssistantMessageIndex < 0
        && currentThinkingIndex < 0
    )
    if (needsTrailingThinkingEntry) {
        startAssistantEntry(turn.timestamp, "", "thinking", turn.status)
    }

    return entries
}

const buildConversationTimelineEntries = (
    snapshot: ConversationSnapshotResponse | null,
    liveTurnEvents: ConversationTurnEventResponse[],
    optimisticSend: OptimisticSendState | null,
): ConversationTimelineEntry[] => {
    if (!snapshot) {
        if (!optimisticSend) {
            return []
        }
        return [
            {
                id: `${optimisticSend.conversationId}:optimistic:user`,
                kind: "message",
                role: "user",
                content: optimisticSend.message,
                timestamp: optimisticSend.createdAt,
                status: "complete",
            },
        ]
    }

    const eventsByTurn = new Map<string, ConversationTurnEventResponse[]>()
    const sortedEvents = sortConversationTurnEvents([...snapshot.turn_events, ...liveTurnEvents])
    sortedEvents.forEach((event) => {
        const entries = eventsByTurn.get(event.turn_id) || []
        entries.push(event)
        eventsByTurn.set(event.turn_id, entries)
    })

    const timeline: ConversationTimelineEntry[] = []
    snapshot.turns.forEach((turn) => {
        if (turn.kind === "spec_edit_proposal" && turn.artifact_id) {
            timeline.push({
                id: turn.id,
                kind: "spec_edit_proposal",
                role: "system",
                artifactId: turn.artifact_id,
                timestamp: turn.timestamp,
            })
            return
        }
        if (turn.kind === "execution_card" && turn.artifact_id) {
            timeline.push({
                id: turn.id,
                kind: "execution_card",
                role: "system",
                artifactId: turn.artifact_id,
                timestamp: turn.timestamp,
            })
            return
        }
        if (turn.role === "user" || turn.role === "assistant") {
            if (turn.role === "assistant") {
                timeline.push(...buildAssistantTimelineEntries(turn, eventsByTurn.get(turn.id) || []))
                return
            }
            timeline.push({
                id: turn.id,
                kind: "message",
                role: turn.role,
                content: turn.content,
                timestamp: turn.timestamp,
                status: turn.status,
                error: turn.error ?? null,
            })
        }
    })

    if (!optimisticSend) {
        return timeline
    }

    const hasMatchingUserTurn = timeline.some((entry) => (
        entry.kind === "message"
        && entry.role === "user"
        && entry.content === optimisticSend.message
        && entry.timestamp >= optimisticSend.createdAt
    ))
    const optimisticEntries: ConversationTimelineEntry[] = []
    if (!hasMatchingUserTurn) {
        optimisticEntries.push({
            id: `${optimisticSend.conversationId}:optimistic:user`,
            kind: "message",
            role: "user",
            content: optimisticSend.message,
            timestamp: optimisticSend.createdAt,
            status: "complete",
        })
    }
    return [...timeline, ...optimisticEntries]
}

const sortConversationSummaries = (items: ConversationSummaryResponse[]) => (
    [...items].sort((left, right) => right.updated_at.localeCompare(left.updated_at))
)

const upsertConversationSummary = (
    items: ConversationSummaryResponse[],
    summary: ConversationSummaryResponse,
) => sortConversationSummaries([
    summary,
    ...items.filter((entry) => entry.conversation_id !== summary.conversation_id),
])

export function HomePanel() {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const projects = Object.values(projectRegistry)
    const recentProjectPaths = useStore((state) => state.recentProjectPaths)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectScopedWorkspaces = useStore((state) => state.projectScopedWorkspaces)
    const projectRegistrationError = useStore((state) => state.projectRegistrationError)
    const registerProject = useStore((state) => state.registerProject)
    const setProjectRegistrationError = useStore((state) => state.setProjectRegistrationError)
    const clearProjectRegistrationError = useStore((state) => state.clearProjectRegistrationError)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setConversationId = useStore((state) => state.setConversationId)
    const appendProjectEventEntry = useStore((state) => state.appendProjectEventEntry)
    const updateProjectScopedWorkspace = useStore((state) => state.updateProjectScopedWorkspace)
    const activeFlow = useStore((state) => state.activeFlow)
    const model = useStore((state) => state.model)

    const [projectGitMetadata, setProjectGitMetadata] = useState<Record<string, ProjectGitMetadata>>({})
    const [projectConversationSnapshots, setProjectConversationSnapshots] = useState<Record<string, ConversationSnapshotResponse>>({})
    const [projectConversationLiveEvents, setProjectConversationLiveEvents] = useState<Record<string, ConversationTurnEventResponse[]>>({})
    const [projectConversationSummaries, setProjectConversationSummaries] = useState<Record<string, ConversationSummaryResponse[]>>({})
    const [chatDraft, setChatDraft] = useState("")
    const [panelError, setPanelError] = useState<string | null>(null)
    const [isSendingChat, setIsSendingChat] = useState(false)
    const [optimisticSend, setOptimisticSend] = useState<OptimisticSendState | null>(null)
    const [pendingSpecProposalId, setPendingSpecProposalId] = useState<string | null>(null)
    const [pendingExecutionCardId, setPendingExecutionCardId] = useState<string | null>(null)
    const [pendingDeleteConversationId, setPendingDeleteConversationId] = useState<string | null>(null)
    const [expandedProposalChanges, setExpandedProposalChanges] = useState<Record<string, boolean>>({})
    const [homeSidebarPrimaryHeight, setHomeSidebarPrimaryHeight] = useState(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)
    const [isHomeSidebarResizing, setIsHomeSidebarResizing] = useState(false)
    const [isConversationPinnedToBottom, setIsConversationPinnedToBottom] = useState(true)

    const projectDirectoryPickerInputRef = useRef<HTMLInputElement | null>(null)
    const homeSidebarRef = useRef<HTMLDivElement | null>(null)
    const homeSidebarResizeRef = useRef<{ startY: number; startHeight: number } | null>(null)
    const conversationBodyRef = useRef<HTMLDivElement | null>(null)
    const applyConversationSnapshotRef = useRef<((projectPath: string, snapshot: ConversationSnapshotResponse, source?: string) => void) | null>(null)
    const applyConversationStreamEventRef = useRef<((projectPath: string, event: ConversationTurnUpsertEventResponse | ConversationTurnEventStreamResponse, source?: string) => void) | null>(null)

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null
    const activeProjectLabel = activeProjectPath ? formatProjectListLabel(activeProjectPath) : null
    const activeProjectGitMetadata = activeProjectPath
        ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
        : EMPTY_PROJECT_GIT_METADATA
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const activeConversationSnapshot = activeConversationId ? projectConversationSnapshots[activeConversationId] || null : null
    const activeConversationLiveEvents = activeConversationId ? projectConversationLiveEvents[activeConversationId] || [] : []
    const activeProjectConversationSummaries = activeProjectPath ? projectConversationSummaries[activeProjectPath] || [] : []
    const activeProjectEventLog = activeProjectScope?.projectEventLog || []
    const activeConversationHistory = useMemo(
        () => buildConversationTimelineEntries(
            activeConversationSnapshot,
            activeConversationLiveEvents,
            optimisticSend && optimisticSend.conversationId === activeConversationId ? optimisticSend : null,
        ),
        [activeConversationId, activeConversationLiveEvents, activeConversationSnapshot, optimisticSend],
    )
    const activeSpecEditProposals = activeConversationSnapshot?.spec_edit_proposals || []
    const activeExecutionCards = activeConversationSnapshot?.execution_cards || []
    const latestSpecEditProposalId = activeSpecEditProposals.length > 0
        ? activeSpecEditProposals[activeSpecEditProposals.length - 1]?.id || null
        : null
    const latestExecutionCardId = activeExecutionCards.length > 0
        ? activeExecutionCards[activeExecutionCards.length - 1]?.id || null
        : null
    const activeSpecEditProposalsById = new Map(activeSpecEditProposals.map((proposal) => [proposal.id, proposal]))
    const activeExecutionCardsById = new Map(activeExecutionCards.map((executionCard) => [executionCard.id, executionCard]))
    const hasRenderableConversationHistory = activeConversationHistory.some((entry) => (
        entry.kind === "spec_edit_proposal"
        || entry.kind === "execution_card"
        || entry.kind === "tool_call"
        || entry.role === "user"
        || entry.role === "assistant"
    ))
    const hasAssistantConversationActivity = activeConversationHistory.some((entry) => (
        entry.kind === "message"
        && entry.role === "assistant"
    ))
    const chatSendButtonLabel = !isSendingChat
        ? "Send"
        : hasAssistantConversationActivity
            ? "Thinking..."
            : "Sending..."

    const orderedProjects = (() => {
        const seenProjectPaths = new Set<string>()
        const items: typeof projects = []

        recentProjectPaths.forEach((projectPath) => {
            const project = projectRegistry[projectPath]
            if (!project || seenProjectPaths.has(projectPath)) {
                return
            }
            items.push(project)
            seenProjectPaths.add(projectPath)
        })

        projects.forEach((project) => {
            if (seenProjectPaths.has(project.directoryPath)) {
                return
            }
            items.push(project)
            seenProjectPaths.add(project.directoryPath)
        })

        return items
    })()

    const appendLocalProjectEvent = (message: string) => {
        appendProjectEventEntry({
            message,
            timestamp: new Date().toISOString(),
        })
    }

    const setProjectConversationSummaryList = (
        projectPath: string,
        summaries: ConversationSummaryResponse[],
    ) => {
        setProjectConversationSummaries((current) => ({
            ...current,
            [projectPath]: sortConversationSummaries(summaries),
        }))
    }

    const persistProjectState = async (
        projectPath: string,
        patch: {
            last_accessed_at?: string | null
            active_conversation_id?: string | null
            is_favorite?: boolean | null
        },
    ) => {
        try {
            const project = await updateProjectStateValidated({
                project_path: projectPath,
                ...patch,
            })
            upsertProjectRegistryEntry(toHydratedProjectRecord(project))
        } catch {
            // Keep the UI responsive if the background state sync fails.
        }
    }

    const activateConversationThread = (projectPath: string, conversationId: string, source = "unknown") => {
        debugProjectChat("activate conversation thread", {
            source,
            projectPath,
            conversationId,
        })
        setOptimisticSend(null)
        setConversationId(conversationId)
        updateProjectScopedWorkspace(projectPath, {
            conversationId,
            specId: null,
            specStatus: "draft",
            specProvenance: null,
            planId: null,
            planStatus: "draft",
            planProvenance: null,
            artifactRunId: null,
        })
        void persistProjectState(projectPath, {
            active_conversation_id: conversationId,
            last_accessed_at: new Date().toISOString(),
        })
    }

    const loadProjectConversationSummaries = async (projectPath: string) => {
        try {
            const summaries = await fetchProjectConversationListValidated(projectPath)
            setProjectConversationSummaryList(projectPath, summaries)
            return summaries
        } catch {
            setProjectConversationSummaryList(projectPath, [])
            return []
        }
    }

    const syncConversationPinnedState = () => {
        const node = conversationBodyRef.current
        if (!node) {
            return
        }
        const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight
        setIsConversationPinnedToBottom(distanceFromBottom <= CONVERSATION_BOTTOM_THRESHOLD_PX)
    }

    const scrollConversationToBottom = () => {
        const node = conversationBodyRef.current
        if (!node) {
            return
        }
        node.scrollTo({
            top: node.scrollHeight,
            behavior: "smooth",
        })
        setIsConversationPinnedToBottom(true)
    }

    const applyConversationSnapshot = (
        projectPath: string,
        snapshot: ConversationSnapshotResponse,
        source = "unknown",
        options?: {
            forceWorkspaceSync?: boolean
        },
    ) => {
        const latestProjectScope = useStore.getState().projectScopedWorkspaces[projectPath]
        const shouldSyncActiveWorkspace = options?.forceWorkspaceSync === true
            || latestProjectScope?.conversationId === snapshot.conversation_id
        const latestApprovedProposal = getLatestApprovedSpecEditProposal(snapshot)
        const latestExecutionCard = getLatestExecutionCard(snapshot)
        const selectedRunId = snapshot.execution_workflow.run_id
            || latestExecutionCard?.source_workflow_run_id
            || latestProjectScope?.selectedRunId
            || null
        const flowSource = snapshot.execution_workflow.flow_source
            || latestExecutionCard?.flow_source
            || latestProjectScope?.activeFlow
            || null
        debugProjectChat("apply conversation snapshot", {
            source,
            projectPath,
            snapshotProjectPath: snapshot.project_path,
            conversationId: snapshot.conversation_id,
            shouldSyncActiveWorkspace,
            turnCount: snapshot.turns.length,
            turns: summarizeConversationTurnsForDebug(snapshot.turns),
        })

        setProjectConversationSnapshots((current) => ({
            ...current,
            [snapshot.conversation_id]: snapshot,
        }))
        setProjectConversationLiveEvents((current) => {
            const existing = current[snapshot.conversation_id] || []
            const nextEvents = pruneLiveConversationTurnEvents(existing, snapshot)
            if (nextEvents.length === 0) {
                if (!(snapshot.conversation_id in current)) {
                    return current
                }
                const next = { ...current }
                delete next[snapshot.conversation_id]
                return next
            }
            return {
                ...current,
                [snapshot.conversation_id]: nextEvents,
            }
        })
        setProjectConversationSummaries((current) => ({
            ...current,
            [projectPath]: upsertConversationSummary(current[projectPath] || [], {
                conversation_id: snapshot.conversation_id,
                project_path: snapshot.project_path,
                title: snapshot.title,
                created_at: snapshot.created_at,
                updated_at: snapshot.updated_at,
                last_message_preview: snapshot.turns
                    .filter((turn) => turn.kind === "message" && typeof turn.content === "string" && turn.content.trim().length > 0)
                    .slice(-1)[0]?.content || null,
            }),
        }))

        if (shouldSyncActiveWorkspace) {
            updateProjectScopedWorkspace(projectPath, {
                conversationId: snapshot.conversation_id,
                projectEventLog: snapshot.event_log.map((entry) => ({
                    message: entry.message,
                    timestamp: entry.timestamp,
                })),
                specId: latestApprovedProposal?.canonical_spec_edit_id ?? null,
                specStatus: latestApprovedProposal ? "approved" : "draft",
                specProvenance: latestApprovedProposal
                    ? {
                        source: "spec-edit-proposal",
                        referenceId: latestApprovedProposal.id,
                        capturedAt: latestApprovedProposal.approved_at || latestApprovedProposal.created_at,
                        runId: null,
                        gitBranch: latestApprovedProposal.git_branch ?? null,
                        gitCommit: latestApprovedProposal.git_commit ?? null,
                    }
                    : null,
                planId: latestExecutionCard?.id ?? null,
                planStatus: derivePlanStatusFromExecutionCard(latestExecutionCard),
                planProvenance: latestExecutionCard
                    ? {
                        source: "execution-card",
                        referenceId: latestExecutionCard.id,
                        capturedAt: latestExecutionCard.updated_at,
                        runId: latestExecutionCard.source_workflow_run_id,
                        gitBranch: latestApprovedProposal?.git_branch ?? null,
                        gitCommit: latestApprovedProposal?.git_commit ?? null,
                    }
                    : null,
                artifactRunId: snapshot.execution_workflow.run_id ?? latestExecutionCard?.source_workflow_run_id ?? null,
                selectedRunId,
                activeFlow: flowSource,
            })
            if (latestProjectScope?.conversationId !== snapshot.conversation_id) {
                void persistProjectState(projectPath, {
                    active_conversation_id: snapshot.conversation_id,
                    last_accessed_at: new Date().toISOString(),
                })
            }
        }

        if (latestApprovedProposal?.git_branch || latestApprovedProposal?.git_commit) {
            setProjectGitMetadata((current) => ({
                ...current,
                [projectPath]: {
                    branch: latestApprovedProposal.git_branch ?? current[projectPath]?.branch ?? null,
                    commit: latestApprovedProposal.git_commit ?? current[projectPath]?.commit ?? null,
                },
            }))
        }
    }

    const applyConversationStreamEvent = (
        projectPath: string,
        event: ConversationTurnUpsertEventResponse | ConversationTurnEventStreamResponse,
        source = "unknown",
    ) => {
        debugProjectChat("apply conversation stream event", {
            source,
            projectPath,
            eventType: event.type,
            conversationId: event.conversation_id,
        })
        let nextSnapshot: ConversationSnapshotResponse | null = null
        setProjectConversationSnapshots((current) => {
            const existingSnapshot = current[event.conversation_id]
                || ensureConversationSnapshotShell(event.conversation_id, event.project_path, event.title)
            let mergedSnapshot = existingSnapshot
            if (event.type === "turn_upsert") {
                const currentTurn = existingSnapshot.turns.find((turn) => turn.id === event.turn.id) || null
                mergedSnapshot = {
                    ...upsertConversationTurn(existingSnapshot, sanitizeStreamingTurnUpsert(currentTurn, event.turn)),
                    project_path: event.project_path,
                    title: event.title,
                    updated_at: event.updated_at,
                }
            } else if (!isTransientConversationTurnEvent(event.event)) {
                mergedSnapshot = {
                    ...appendConversationTurnEvent(existingSnapshot, event.event),
                    project_path: event.project_path,
                    title: event.title,
                    updated_at: event.updated_at,
                }
            } else {
                mergedSnapshot = {
                    ...existingSnapshot,
                    project_path: event.project_path,
                    title: event.title,
                    updated_at: event.updated_at,
                }
            }
            nextSnapshot = mergedSnapshot
            return {
                ...current,
                [event.conversation_id]: mergedSnapshot,
            }
        })
        if (event.type === "turn_event" && isTransientConversationTurnEvent(event.event)) {
            setProjectConversationLiveEvents((current) => ({
                ...current,
                [event.conversation_id]: appendLiveConversationTurnEvent(current[event.conversation_id] || [], event.event),
            }))
        }
        if (!nextSnapshot) {
            return
        }
        setProjectConversationSummaries((current) => ({
            ...current,
            [projectPath]: upsertConversationSummary(current[projectPath] || [], {
                conversation_id: nextSnapshot.conversation_id,
                project_path: nextSnapshot.project_path,
                title: nextSnapshot.title,
                created_at: nextSnapshot.created_at,
                updated_at: nextSnapshot.updated_at,
                last_message_preview: nextSnapshot.turns
                    .filter((turn) => turn.kind === "message" && turn.content.trim().length > 0)
                    .slice(-1)[0]?.content || null,
            }),
        }))
    }

    applyConversationSnapshotRef.current = applyConversationSnapshot
    applyConversationStreamEventRef.current = applyConversationStreamEvent

    useEffect(() => {
        let isCancelled = false

        const loadProjectRegistry = async () => {
            try {
                const projects = await fetchProjectRegistryValidated()
                if (isCancelled) {
                    return
                }
                hydrateProjectRegistry(projects.map(toHydratedProjectRecord))
            } catch {
                // Leave the in-memory registry untouched if the initial load fails.
            }
        }

        void loadProjectRegistry()
        return () => {
            isCancelled = true
        }
    }, [hydrateProjectRegistry])

    useEffect(() => {
        const projectPathsToFetch = projects
            .map((project) => project.directoryPath)
            .filter((projectPath) => !(projectPath in projectGitMetadata))
        if (projectPathsToFetch.length === 0) {
            return
        }

        let isCancelled = false
        const loadBranches = async () => {
            const entries = await Promise.all(
                projectPathsToFetch.map(async (projectPath) => {
                    try {
                        const response = await fetch(`/api/projects/metadata?directory=${encodeURIComponent(projectPath)}`)
                        if (!response.ok) {
                            return [projectPath, { ...EMPTY_PROJECT_GIT_METADATA }] as const
                        }
                        const payload = (await response.json()) as { branch?: string | null; commit?: string | null }
                        return [
                            projectPath,
                            {
                                branch: asProjectGitMetadataField(payload.branch),
                                commit: asProjectGitMetadataField(payload.commit),
                            },
                        ] as const
                    } catch {
                        return [projectPath, { ...EMPTY_PROJECT_GIT_METADATA }] as const
                    }
                }),
            )

            if (isCancelled) {
                return
            }

            setProjectGitMetadata((current) => {
                const next = { ...current }
                entries.forEach(([projectPath, metadata]) => {
                    next[projectPath] = metadata
                })
                return next
            })
        }

        void loadBranches()
        return () => {
            isCancelled = true
        }
    }, [projectGitMetadata, projects])

    useEffect(() => {
        if (!activeProjectPath) {
            return
        }

        let isCancelled = false
        const loadThreadSummaries = async () => {
            const summaries = await loadProjectConversationSummaries(activeProjectPath)
            if (isCancelled) {
                return
            }
            if (activeConversationId) {
                return
            }
            const latestConversation = summaries[0] || null
            if (latestConversation) {
                activateConversationThread(activeProjectPath, latestConversation.conversation_id, "load-latest-thread")
            }
        }

        void loadThreadSummaries()
        return () => {
            isCancelled = true
        }
    }, [activeConversationId, activeProjectPath])

    useEffect(() => {
        setChatDraft("")
        setPanelError(null)
        setOptimisticSend(null)
    }, [activeProjectPath])

    useEffect(() => {
        setExpandedProposalChanges({})
    }, [activeProjectPath, latestSpecEditProposalId])

    useEffect(() => {
        if (!projectDirectoryPickerInputRef.current) {
            return
        }
        projectDirectoryPickerInputRef.current.setAttribute("webkitdirectory", "")
        projectDirectoryPickerInputRef.current.setAttribute("directory", "")
    }, [])

    useEffect(() => {
        if (isNarrowViewport) {
            setIsHomeSidebarResizing(false)
            homeSidebarResizeRef.current = null
            return
        }

        const syncSidebarHeight = () => {
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setHomeSidebarPrimaryHeight((current) => clampHomeSidebarPrimaryHeight(current, containerHeight))
        }

        syncSidebarHeight()
        window.addEventListener("resize", syncSidebarHeight)
        return () => {
            window.removeEventListener("resize", syncSidebarHeight)
        }
    }, [isNarrowViewport])

    useEffect(() => {
        if (!isHomeSidebarResizing) {
            return
        }

        const stopHomeSidebarResize = () => {
            setIsHomeSidebarResizing(false)
            homeSidebarResizeRef.current = null
            document.body.style.cursor = ""
            document.body.style.userSelect = ""
        }

        const handleHomeSidebarPointerMove = (event: PointerEvent) => {
            const resizeState = homeSidebarResizeRef.current
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (!resizeState || containerHeight <= 0) {
                return
            }
            const nextHeight = resizeState.startHeight + (event.clientY - resizeState.startY)
            setHomeSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(nextHeight, containerHeight))
        }

        window.addEventListener("pointermove", handleHomeSidebarPointerMove)
        window.addEventListener("pointerup", stopHomeSidebarResize)
        window.addEventListener("pointercancel", stopHomeSidebarResize)
        return () => {
            window.removeEventListener("pointermove", handleHomeSidebarPointerMove)
            window.removeEventListener("pointerup", stopHomeSidebarResize)
            window.removeEventListener("pointercancel", stopHomeSidebarResize)
            document.body.style.cursor = ""
            document.body.style.userSelect = ""
        }
    }, [isHomeSidebarResizing])

    useEffect(() => {
        setIsConversationPinnedToBottom(true)
    }, [activeProjectPath])

    useEffect(() => {
        if (!isConversationPinnedToBottom) {
            return
        }
        const frame = window.requestAnimationFrame(() => {
            const node = conversationBodyRef.current
            if (!node) {
                return
            }
            node.scrollTop = node.scrollHeight
        })
        return () => {
            window.cancelAnimationFrame(frame)
        }
    }, [activeConversationHistory, activeProjectPath, isConversationPinnedToBottom])

    useEffect(() => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        let isCancelled = false
        let eventSource: EventSource | null = null

        const loadSnapshot = async () => {
            try {
                const snapshot = await fetchConversationSnapshotValidated(activeConversationId, activeProjectPath)
                if (isCancelled) {
                    return
                }
                applyConversationSnapshotRef.current?.(activeProjectPath, snapshot, "snapshot-fetch")
            } catch (error) {
                if (isCancelled) {
                    return
                }
                if (error instanceof ApiHttpError && error.status === 404) {
                    return
                }
                const message = extractApiErrorMessage(error, "Unable to load project conversation.")
                setPanelError(message)
                appendLocalProjectEvent(`Project chat sync failed: ${message}`)
            }
        }

        void loadSnapshot()

        if (typeof EventSource !== "undefined") {
            const eventStreamUrl = `/api/conversations/${encodeURIComponent(activeConversationId)}/events?project_path=${encodeURIComponent(activeProjectPath)}`
            eventSource = new EventSource(eventStreamUrl)
            eventSource.onmessage = (event) => {
                if (isCancelled) {
                    return
                }
                try {
                    const payload = JSON.parse(event.data) as { type?: string; state?: unknown }
                    if (payload.type === "conversation_snapshot") {
                        const snapshot = parseConversationSnapshotResponse(payload.state, "/api/conversations/{id}/events")
                        applyConversationSnapshotRef.current?.(activeProjectPath, snapshot, "event-stream-snapshot")
                        return
                    }
                    const parsedEvent = parseConversationStreamEventResponse(payload, "/api/conversations/{id}/events")
                    if (!parsedEvent) {
                        return
                    }
                    applyConversationStreamEventRef.current?.(activeProjectPath, parsedEvent, "event-stream")
                } catch {
                    // Ignore malformed stream events.
                }
            }
        }

        return () => {
            isCancelled = true
            eventSource?.close()
        }
    }, [activeConversationId, activeProjectPath])

    const resolveProjectPathValidation = (rawPath: string): ProjectRegistrationResult => {
        const normalizedPath = normalizeProjectPath(rawPath)
        if (!normalizedPath) {
            return { ok: false, error: "Project directory path is required." }
        }
        if (!isAbsoluteProjectPath(normalizedPath)) {
            return {
                ok: false,
                normalizedPath,
                error: "Project directory path must be absolute.",
            }
        }
        const duplicate = Boolean(projectRegistry[normalizedPath])
        if (duplicate) {
            return {
                ok: false,
                normalizedPath,
                error: `Project already registered: ${normalizedPath}`,
            }
        }
        return {
            ok: true,
            normalizedPath,
        }
    }

    const fetchProjectGitMetadata = async (
        projectPath: string,
    ): Promise<{ metadata: ProjectGitMetadata; error?: string }> => {
        try {
            const response = await fetch(`/api/projects/metadata?directory=${encodeURIComponent(projectPath)}`)
            if (!response.ok) {
                let message = "Unable to verify project Git state."
                try {
                    const payload = (await response.json()) as { detail?: string }
                    if (payload.detail) {
                        message = payload.detail
                    }
                } catch {
                    // Ignore malformed error payloads.
                }
                return { metadata: { ...EMPTY_PROJECT_GIT_METADATA }, error: message }
            }
            const payload = (await response.json()) as { branch?: string | null; commit?: string | null }
            return {
                metadata: {
                    branch: asProjectGitMetadataField(payload.branch),
                    commit: asProjectGitMetadataField(payload.commit),
                },
            }
        } catch {
            return { metadata: { ...EMPTY_PROJECT_GIT_METADATA }, error: "Unable to verify project Git state." }
        }
    }

    const ensureProjectGitRepository = async (projectPath: string): Promise<ProjectGitMetadata | null> => {
        const { metadata, error } = await fetchProjectGitMetadata(projectPath)
        setProjectGitMetadata((current) => ({ ...current, [projectPath]: metadata }))
        if (error) {
            setProjectRegistrationError(error)
            return null
        }
        if (!metadata.branch && !metadata.commit) {
            setProjectRegistrationError("Project directory must be a Git repository.")
            return null
        }
        return metadata
    }

    const registerProjectFromPath = async (rawProjectPath: string) => {
        const validation = resolveProjectPathValidation(rawProjectPath)
        if (!validation.ok || !validation.normalizedPath) {
            setProjectRegistrationError(validation.error ?? "Project directory path is required.")
            return
        }
        const gitMetadata = await ensureProjectGitRepository(validation.normalizedPath)
        if (!gitMetadata) {
            return
        }
        const result = registerProject(validation.normalizedPath)
        if (!result.ok) {
            setProjectRegistrationError(result.error ?? "Unable to register the project.")
            return
        }
        try {
            const projectRecord = await registerProjectValidated(validation.normalizedPath)
            upsertProjectRegistryEntry(toHydratedProjectRecord(projectRecord))
        } catch (error) {
            useStore.setState((state) => {
                const nextProjectRegistry = { ...state.projectRegistry }
                const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
                delete nextProjectRegistry[validation.normalizedPath]
                delete nextProjectScopedWorkspaces[validation.normalizedPath]
                const nextActiveProjectPath = state.activeProjectPath === validation.normalizedPath ? null : state.activeProjectPath
                return {
                    projectRegistry: nextProjectRegistry,
                    projectScopedWorkspaces: nextProjectScopedWorkspaces,
                    activeProjectPath: nextActiveProjectPath,
                    activeFlow: nextActiveProjectPath ? state.activeFlow : null,
                    selectedRunId: nextActiveProjectPath ? state.selectedRunId : null,
                    workingDir: nextActiveProjectPath ? state.workingDir : "./test-app",
                }
            })
            setProjectRegistrationError(extractApiErrorMessage(error, "Unable to register the project."))
            return
        }
        if (result.ok) {
            setProjectRegistrationError(null)
        }
    }

    const onOpenProjectDirectoryChooser = async () => {
        clearProjectRegistrationError()
        try {
            const selection = await pickProjectDirectoryValidated()
            if (selection.status === "canceled") {
                return
            }
            await registerProjectFromPath(selection.directory_path)
            return
        } catch (error) {
            const canUseBrowserFallback = error instanceof ApiHttpError
                && [404, 405, 501, 503].includes(error.status)
                && projectDirectoryPickerInputRef.current
            if (!canUseBrowserFallback) {
                setProjectRegistrationError(extractApiErrorMessage(error, "Directory picker is unavailable."))
                return
            }
        }
        if (!projectDirectoryPickerInputRef.current) {
            setProjectRegistrationError("Directory picker is unavailable.")
            return
        }
        projectDirectoryPickerInputRef.current.value = ""
        projectDirectoryPickerInputRef.current.click()
    }

    const adjustHomeSidebarPrimaryHeight = (delta: number) => {
        const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
        if (containerHeight <= 0) {
            return
        }
        setHomeSidebarPrimaryHeight((current) => clampHomeSidebarPrimaryHeight(current + delta, containerHeight))
    }

    const onHomeSidebarResizePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
        if (isNarrowViewport) {
            return
        }
        homeSidebarResizeRef.current = {
            startY: event.clientY,
            startHeight: homeSidebarPrimaryHeight,
        }
        setIsHomeSidebarResizing(true)
        document.body.style.cursor = "row-resize"
        document.body.style.userSelect = "none"
        event.preventDefault()
    }

    const onHomeSidebarResizeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === "ArrowUp") {
            event.preventDefault()
            adjustHomeSidebarPrimaryHeight(-24)
            return
        }
        if (event.key === "ArrowDown") {
            event.preventDefault()
            adjustHomeSidebarPrimaryHeight(24)
            return
        }
        if (event.key === "Home") {
            event.preventDefault()
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setHomeSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(HOME_SIDEBAR_MIN_PRIMARY_HEIGHT, containerHeight))
            return
        }
        if (event.key === "End") {
            event.preventDefault()
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setHomeSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(containerHeight, containerHeight))
        }
    }

    const onProjectDirectorySelected = (event: ChangeEvent<HTMLInputElement>) => {
        const files = event.target.files
        const selectedProjectPath = deriveProjectPathFromDirectorySelection(files)
        event.target.value = ""
        if (!selectedProjectPath) {
            setProjectRegistrationError(
                "Unable to resolve an absolute project path from the selected directory.",
            )
            return
        }
        void registerProjectFromPath(selectedProjectPath)
    }

    const onActivateProject = async (projectPath: string) => {
        if (!projectPath) {
            return
        }
        if (projectPath === activeProjectPath) {
            setActiveProjectPath(projectPath)
            return
        }
        const gitMetadata = await ensureProjectGitRepository(projectPath)
        if (!gitMetadata) {
            return
        }
        setProjectRegistrationError(null)
        setActiveProjectPath(projectPath)
        void persistProjectState(projectPath, {
            last_accessed_at: new Date().toISOString(),
        })
    }

    const onCreateConversationThread = () => {
        if (!activeProjectPath) {
            return
        }
        const now = new Date().toISOString()
        const conversationId = buildProjectConversationId(activeProjectPath)
        setPanelError(null)
        setProjectConversationSummaryList(activeProjectPath, upsertConversationSummary(
            projectConversationSummaries[activeProjectPath] || [],
            {
                conversation_id: conversationId,
                project_path: activeProjectPath,
                title: "New thread",
                created_at: now,
                updated_at: now,
                last_message_preview: null,
            },
        ))
        activateConversationThread(activeProjectPath, conversationId, "create-thread")
    }

    const onSelectConversationThread = (conversationId: string) => {
        if (!activeProjectPath) {
            return
        }
        setPanelError(null)
        activateConversationThread(activeProjectPath, conversationId, "select-thread")
        const cachedSnapshot = projectConversationSnapshots[conversationId]
        if (cachedSnapshot) {
            applyConversationSnapshot(activeProjectPath, cachedSnapshot, "thread-cache")
        }
    }

    const onDeleteConversationThread = async (conversationId: string, title: string) => {
        if (!activeProjectPath) {
            return
        }
        if (typeof window !== "undefined" && !window.confirm(`Delete thread "${title}"?`)) {
            return
        }
        setPanelError(null)
        setPendingDeleteConversationId(conversationId)
        try {
            await deleteConversationValidated(conversationId, activeProjectPath)
            setProjectConversationSnapshots((current) => {
                const next = { ...current }
                delete next[conversationId]
                return next
            })
            const localRemainingSummaries = sortConversationSummaries(
                (projectConversationSummaries[activeProjectPath] || []).filter(
                    (entry) => entry.conversation_id !== conversationId,
                ),
            )
            setProjectConversationSummaryList(activeProjectPath, localRemainingSummaries)

            let remainingSummaries = localRemainingSummaries
            try {
                remainingSummaries = await fetchProjectConversationListValidated(activeProjectPath)
                setProjectConversationSummaryList(activeProjectPath, remainingSummaries)
            } catch {
                // Keep the local optimistic removal if the follow-up refresh fails.
            }

            if (activeConversationId === conversationId) {
                const fallbackConversationId = remainingSummaries[0]?.conversation_id || null
                setOptimisticSend(null)
                setConversationId(fallbackConversationId)
                if (fallbackConversationId) {
                    updateProjectScopedWorkspace(activeProjectPath, {
                        conversationId: fallbackConversationId,
                    })
                }
                void persistProjectState(activeProjectPath, {
                    active_conversation_id: fallbackConversationId,
                    last_accessed_at: new Date().toISOString(),
                })
            }
        } catch (error) {
            const message = extractApiErrorMessage(error, "Unable to delete the thread.")
            setPanelError(message)
            appendLocalProjectEvent(`Thread deletion failed: ${message}`)
        } finally {
            setPendingDeleteConversationId(null)
        }
    }

    const ensureConversationId = () => {
        if (!activeProjectPath) {
            return null
        }
        if (activeConversationId) {
            return activeConversationId
        }
        const conversationId = buildProjectConversationId(activeProjectPath)
        activateConversationThread(activeProjectPath, conversationId, "ensure-conversation")
        return conversationId
    }

    const onSendChatMessage = async () => {
        if (!activeProjectPath) {
            return
        }
        const trimmed = chatDraft.trim()
        if (!trimmed) {
            return
        }
        const conversationId = ensureConversationId()
        if (!conversationId) {
            return
        }
        const optimisticCreatedAt = new Date().toISOString()

        setIsSendingChat(true)
        setPanelError(null)
        setChatDraft("")
        setOptimisticSend({
            conversationId,
            projectPath: activeProjectPath,
            message: trimmed,
            createdAt: optimisticCreatedAt,
        })
        try {
            const snapshot = await sendConversationTurnValidated(conversationId, {
                project_path: activeProjectPath,
                message: trimmed,
                model: model.trim() || null,
            })
            const latestProjectScope = useStore.getState().projectScopedWorkspaces[activeProjectPath]
            const shouldKeepFocusOnReplyThread = latestProjectScope?.conversationId === conversationId
            applyConversationSnapshot(activeProjectPath, snapshot, "send-response", {
                forceWorkspaceSync: shouldKeepFocusOnReplyThread,
            })
        } catch (error) {
            const message = extractApiErrorMessage(error, "Unable to send the project chat turn.")
            setPanelError(message)
            appendLocalProjectEvent(`Project chat turn failed: ${message}`)
        } finally {
            setIsSendingChat(false)
            setOptimisticSend(null)
        }
    }

    const onChatComposerSubmit = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        void onSendChatMessage()
    }

    const onChatComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
        if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault()
            void onSendChatMessage()
        }
    }

    const onApproveSpecEditProposal = async (proposal: SpecEditProposalResponse) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }
        if (!window.confirm("Approve these spec edits, commit them to git, and start execution planning?")) {
            return
        }

        setPendingSpecProposalId(proposal.id)
        setPanelError(null)
        try {
            const snapshot = await approveSpecEditProposalValidated(activeConversationId, proposal.id, {
                project_path: activeProjectPath,
                model: model.trim() || null,
                flow_source: activeFlow || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, "spec-approve")
        } catch (error) {
            const message = extractApiErrorMessage(error, "Unable to approve the spec edit proposal.")
            setPanelError(message)
            appendLocalProjectEvent(`Spec edit approval failed: ${message}`)
        } finally {
            setPendingSpecProposalId(null)
        }
    }

    const onRejectSpecEditProposal = async (proposal: SpecEditProposalResponse) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        setPendingSpecProposalId(proposal.id)
        setPanelError(null)
        try {
            const snapshot = await rejectSpecEditProposalValidated(activeConversationId, proposal.id, {
                project_path: activeProjectPath,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, "spec-reject")
        } catch (error) {
            const message = extractApiErrorMessage(error, "Unable to reject the spec edit proposal.")
            setPanelError(message)
            appendLocalProjectEvent(`Spec edit rejection failed: ${message}`)
        } finally {
            setPendingSpecProposalId(null)
        }
    }

    const onReviewExecutionCard = async (
        executionCard: ExecutionCardResponse,
        disposition: "approved" | "rejected" | "revision_requested",
    ) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        const reviewMessage = disposition === "approved"
            ? "Approved for dispatch."
            : window.prompt(
                disposition === "revision_requested"
                    ? "Describe what should change before execution planning is regenerated."
                    : "Describe why this execution card should be rejected.",
                "",
            )?.trim() || ""

        if (!reviewMessage) {
            return
        }

        setPendingExecutionCardId(executionCard.id)
        setPanelError(null)
        try {
            const snapshot = await reviewExecutionCardValidated(activeConversationId, executionCard.id, {
                project_path: activeProjectPath,
                disposition,
                message: reviewMessage,
                model: model.trim() || null,
                flow_source: activeFlow || executionCard.flow_source || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, "execution-review")
        } catch (error) {
            const message = extractApiErrorMessage(error, "Unable to review the execution card.")
            setPanelError(message)
            appendLocalProjectEvent(`Execution card review failed: ${message}`)
        } finally {
            setPendingExecutionCardId(null)
        }
    }

    const formatConversationTimestamp = (value: string) => {
        const parsed = new Date(value)
        if (Number.isNaN(parsed.getTime())) {
            return value
        }
        return parsed.toLocaleString()
    }

    const buildProposalDiffLines = (change: SpecEditProposalResponse["changes"][number]) => {
        const beforeLines = change.before.split("\n").map((line) => ({ type: "removed" as const, text: line }))
        const afterLines = change.after.split("\n").map((line) => ({ type: "added" as const, text: line }))
        return [...beforeLines, ...afterLines]
    }

    const buildProposalChangeKey = (proposalId: string, changePath: string, index: number) => (
        `${proposalId}:${changePath}:${index}`
    )

    const toggleProposalChangeExpanded = (changeKey: string) => {
        setExpandedProposalChanges((current) => ({
            ...current,
            [changeKey]: !current[changeKey],
        }))
    }

    return (
        <section
            data-testid="projects-panel"
            data-home-panel="true"
            data-responsive-layout={isNarrowViewport ? "stacked" : "split"}
            className={`flex-1 ${isNarrowViewport ? "overflow-auto p-3" : "flex min-h-0 flex-col overflow-hidden p-6"}`}
        >
            <div className={`w-full ${isNarrowViewport ? "space-y-6" : "flex min-h-0 flex-1 flex-col gap-6"}`}>
                <div
                    data-testid="home-main-layout"
                    className={`grid gap-4 ${isNarrowViewport ? "grid-cols-1" : "min-h-0 flex-1 grid-cols-[minmax(18rem,22rem)_minmax(0,1fr)]"}`}
                >
                    <HomeProjectSidebar className={isNarrowViewport ? "gap-4" : "h-full"}>
                        <div
                            ref={homeSidebarRef}
                            data-testid="home-sidebar-stack"
                            className={`flex ${isNarrowViewport ? "flex-col gap-4" : "h-full min-h-0 flex-col"}`}
                        >
                            <div
                                data-testid="home-sidebar-primary-surface"
                                className={`rounded-md border border-border bg-card shadow-sm ${isNarrowViewport ? "" : "min-h-0 overflow-hidden"}`}
                                style={isNarrowViewport ? undefined : { height: `${homeSidebarPrimaryHeight}px` }}
                            >
                                <div className="flex h-full min-h-0 flex-col p-4">
                                    <div className="mb-3 space-y-2">
                                        <div
                                            data-testid="quick-switch-controls"
                                            data-responsive-layout={isNarrowViewport ? "stacked" : "inline"}
                                            className={`items-start justify-between gap-2 ${isNarrowViewport ? "flex flex-col" : "flex"}`}
                                        >
                                            <h3 className="text-sm font-semibold text-foreground">Projects</h3>
                                            <button
                                                data-testid="quick-switch-new-button"
                                                type="button"
                                                onClick={onOpenProjectDirectoryChooser}
                                                className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            >
                                                New
                                            </button>
                                        </div>
                                        <input
                                            ref={projectDirectoryPickerInputRef}
                                            data-testid="project-directory-picker-input"
                                            type="file"
                                            multiple
                                            onChange={onProjectDirectorySelected}
                                            className="hidden"
                                            tabIndex={-1}
                                            aria-hidden="true"
                                        />
                                        {projectRegistrationError ? (
                                            <p data-testid="project-registration-error" className="text-xs text-destructive">
                                                {projectRegistrationError}
                                            </p>
                                        ) : null}
                                    </div>
                                    <div className={isNarrowViewport ? "" : "min-h-0 flex-1 overflow-y-auto pr-1"}>
                                        <ul data-testid="projects-list" className="space-y-1.5">
                                            {orderedProjects.length === 0 ? (
                                                <li className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                                                    No projects registered yet.
                                                </li>
                                            ) : (
                                                orderedProjects.map((project) => {
                                                    const projectPath = project.directoryPath
                                                    const isActive = projectPath === activeProjectPath
                                                    const projectConversationSummaries = isActive ? activeProjectConversationSummaries : []
                                                    return (
                                                        <li key={projectPath} className="space-y-1">
                                                            <button
                                                                type="button"
                                                                onClick={() => {
                                                                    void onActivateProject(projectPath)
                                                                }}
                                                                aria-current={isActive ? "true" : undefined}
                                                                title={projectPath}
                                                                className={`w-full rounded-md px-2 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActive
                                                                    ? "bg-primary/10 text-foreground"
                                                                    : "text-foreground hover:bg-muted/70"
                                                                    }`}
                                                            >
                                                                <div className="flex items-center gap-2">
                                                                    {isActive ? (
                                                                        <FolderOpen className="h-4 w-4 shrink-0 text-primary" />
                                                                    ) : (
                                                                        <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                                                                    )}
                                                                    <span className={`truncate text-sm font-medium ${isActive ? "text-foreground" : "text-foreground/90"}`}>
                                                                        {formatProjectListLabel(projectPath)}
                                                                    </span>
                                                                </div>
                                                            </button>
                                                            {isActive ? (
                                                                <div className="ml-5 border-l border-border/70 pl-2">
                                                                    <div
                                                                        data-testid="project-thread-controls"
                                                                        className="mb-1 flex justify-end"
                                                                    >
                                                                        <button
                                                                            data-testid="project-thread-new-button"
                                                                            type="button"
                                                                            onClick={onCreateConversationThread}
                                                                            className="inline-flex items-center gap-1 rounded px-1.5 py-1 text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                                        >
                                                                            <Plus className="h-3.5 w-3.5" />
                                                                            <span>New thread</span>
                                                                        </button>
                                                                    </div>
                                                                    <ul data-testid="project-thread-list" className="space-y-1">
                                                                        {projectConversationSummaries.length === 0 ? (
                                                                            <li className="px-2 py-1 text-[11px] text-muted-foreground">
                                                                                No threads yet.
                                                                            </li>
                                                                        ) : (
                                                                            projectConversationSummaries.map((conversation) => {
                                                                                const isActiveConversation = conversation.conversation_id === activeConversationId
                                                                                const ageLabel = formatConversationAgeShort(conversation.updated_at)
                                                                                const isDeletingConversation = pendingDeleteConversationId === conversation.conversation_id
                                                                                return (
                                                                                    <li key={conversation.conversation_id} className="group/thread relative">
                                                                                        <button
                                                                                            type="button"
                                                                                            onClick={() => onSelectConversationThread(conversation.conversation_id)}
                                                                                            aria-current={isActiveConversation ? "true" : undefined}
                                                                                            aria-label={`Open thread ${conversation.title}`}
                                                                                            className={`w-full rounded-xl px-2 py-2 pr-9 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActiveConversation
                                                                                                ? "bg-muted text-foreground shadow-sm"
                                                                                                : "text-foreground/90 hover:bg-muted/60"
                                                                                                }`}
                                                                                        >
                                                                                            <div className="flex items-center gap-2">
                                                                                                <FileText className={`h-3.5 w-3.5 shrink-0 ${isActiveConversation ? "text-foreground" : "text-muted-foreground"}`} />
                                                                                                <span className="min-w-0 flex-1 truncate text-[13px] font-medium">
                                                                                                    {conversation.title}
                                                                                                </span>
                                                                                                <span className="shrink-0 text-[11px] text-muted-foreground transition-opacity group-hover/thread:opacity-0 group-focus-within/thread:opacity-0">
                                                                                                    {ageLabel}
                                                                                                </span>
                                                                                            </div>
                                                                                        </button>
                                                                                        <button
                                                                                            type="button"
                                                                                            aria-label={`Delete thread ${conversation.title}`}
                                                                                            data-testid={`project-thread-delete-${conversation.conversation_id}`}
                                                                                            onClick={() => {
                                                                                                void onDeleteConversationThread(conversation.conversation_id, conversation.title)
                                                                                            }}
                                                                                            disabled={isDeletingConversation}
                                                                                            className="absolute right-1 top-1/2 inline-flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-destructive focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring group-hover/thread:opacity-100 group-focus-within/thread:opacity-100 disabled:cursor-not-allowed disabled:opacity-50"
                                                                                        >
                                                                                            <Trash2 className="h-3.5 w-3.5" />
                                                                                        </button>
                                                                                    </li>
                                                                                )
                                                                            })
                                                                        )}
                                                                    </ul>
                                                                </div>
                                                            ) : null}
                                                        </li>
                                                    )
                                                })
                                            )}
                                        </ul>
                                    </div>
                                </div>
                            </div>
                            {!isNarrowViewport ? (
                                <div
                                    data-testid="home-sidebar-resize-handle"
                                    role="separator"
                                    aria-label="Resize sidebar sections"
                                    aria-orientation="horizontal"
                                    tabIndex={0}
                                    onPointerDown={onHomeSidebarResizePointerDown}
                                    onKeyDown={onHomeSidebarResizeKeyDown}
                                    className={`group flex h-3 shrink-0 cursor-row-resize items-center justify-center rounded-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isHomeSidebarResizing ? "bg-muted" : "hover:bg-muted/60"}`}
                                >
                                    <span className="h-1 w-12 rounded-full bg-border transition-colors group-hover:bg-muted-foreground/70" />
                                </div>
                            ) : null}
                            <div
                                data-testid="project-event-log-surface"
                                className={`flex min-h-[280px] flex-col rounded-md border border-border bg-card p-4 shadow-sm ${isNarrowViewport ? "" : "min-h-0 flex-1 overflow-hidden"}`}
                            >
                                <div className="mb-3 space-y-1">
                                    <h3 className="text-sm font-semibold text-foreground">Workflow Event Log</h3>
                                    <p className="text-xs text-muted-foreground">
                                        Project-scoped operational events and workflow progression.
                                    </p>
                                </div>
                                {!activeProjectPath ? (
                                    <p className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                                        Select a project to view workflow events.
                                    </p>
                                ) : activeProjectEventLog.length === 0 ? (
                                    <p className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                                        No workflow events recorded for this project yet.
                                    </p>
                                ) : (
                                    <ol data-testid="project-event-log-list" className="flex-1 space-y-2 overflow-y-auto pr-1">
                                        {[...activeProjectEventLog].reverse().map((entry, index) => (
                                            <li key={`${entry.timestamp}-${index}`} className="rounded border border-border px-2 py-1.5">
                                                <p className="text-[10px] text-muted-foreground">{formatConversationTimestamp(entry.timestamp)}</p>
                                                <p className="text-xs text-foreground">{entry.message}</p>
                                            </li>
                                        ))}
                                    </ol>
                                )}
                            </div>
                        </div>
                    </HomeProjectSidebar>
                    <HomeWorkspace className={isNarrowViewport ? "space-y-4" : "h-full"}>
                        <div
                            data-testid="project-ai-conversation-surface"
                            className={`rounded-md border border-border bg-card p-4 shadow-sm ${isNarrowViewport ? "" : "flex h-full min-h-0 flex-col"}`}
                        >
                            <p className="mb-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                                {activeProjectLabel ? `Project Chat - ${activeProjectLabel}` : "Project Chat"}
                            </p>
                            {!activeProjectPath ? (
                                <p className={`rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground ${isNarrowViewport ? "" : "flex flex-1 items-center"}`}>
                                    Select an active project to begin chatting.
                                </p>
                            ) : (
                                <div className="flex min-h-0 flex-1 flex-col gap-3">
                                    <div
                                        ref={conversationBodyRef}
                                        data-testid="project-ai-conversation-body"
                                        onScroll={syncConversationPinnedState}
                                        className={`flex min-h-0 flex-1 flex-col gap-3 ${isNarrowViewport ? "" : "overflow-y-auto pr-1"}`}
                                    >
                                        <div data-testid="project-ai-conversation-history" className="flex min-h-0 flex-col">
                                            {!hasRenderableConversationHistory ? (
                                                <p className="rounded-md border border-dashed border-border px-3 py-4 text-sm text-muted-foreground">
                                                    {activeConversationId
                                                        ? "No conversation history for this thread yet."
                                                        : "Create or select a thread to begin chatting."}
                                                </p>
                                            ) : (
                                                <ol data-testid="project-ai-conversation-history-list" className="space-y-3">
                                                    {activeConversationHistory.map((entry, index) => {
                                                        const key = `${entry.id}-${entry.timestamp}-${index}`
                                                        if (entry.kind === "tool_call") {
                                                            const statusPresentation = getToolCallStatusPresentation(entry.toolCall.status)
                                                            return (
                                                                <li key={key} className="flex justify-start">
                                                                    <div className="w-full rounded-md border border-border bg-muted/40 px-3 py-2">
                                                                        <div className="flex flex-wrap items-center gap-2">
                                                                            <p className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                                                                {entry.toolCall.kind === "file_change" ? "File change" : "Tool call"}
                                                                            </p>
                                                                            <span className={getSurfaceToneClassName(statusPresentation.tone)}>
                                                                                {statusPresentation.label}
                                                                            </span>
                                                                            <p className="text-xs font-medium text-foreground">{entry.toolCall.title}</p>
                                                                        </div>
                                                                        {entry.toolCall.command ? (
                                                                            <p className="mt-2 whitespace-pre-wrap rounded border border-border/60 bg-background/80 px-2 py-1 font-mono text-[11px] text-foreground">
                                                                                {entry.toolCall.command}
                                                                            </p>
                                                                        ) : null}
                                                                        {entry.toolCall.filePaths.length > 0 ? (
                                                                            <ul className="mt-2 space-y-1">
                                                                                {entry.toolCall.filePaths.map((path) => (
                                                                                    <li key={`${key}-${path}`} className="font-mono text-[11px] text-muted-foreground">
                                                                                        {path}
                                                                                    </li>
                                                                                ))}
                                                                            </ul>
                                                                        ) : null}
                                                                        {entry.toolCall.output ? (
                                                                            <pre className="mt-2 max-h-40 overflow-auto rounded border border-border/60 bg-background/80 px-2 py-1 whitespace-pre-wrap font-mono text-[11px] text-muted-foreground">
                                                                                {entry.toolCall.output}
                                                                            </pre>
                                                                        ) : null}
                                                                    </div>
                                                                </li>
                                                            )
                                                        }

                                                        if (entry.kind === "spec_edit_proposal") {
                                                            const proposal = activeSpecEditProposalsById.get(entry.artifactId) || null
                                                            if (!proposal) {
                                                                return (
                                                                    <li key={key} className="flex justify-start">
                                                                        <div className="w-full rounded-md border border-border bg-muted/40 px-4 py-3 text-xs text-muted-foreground">
                                                                            Spec edit artifact unavailable. Refresh the project chat to reload it.
                                                                        </div>
                                                                    </li>
                                                                )
                                                            }
                                                            const statusPresentation = getSpecEditStatusPresentation(proposal.status)
                                                            const isLatestProposal = proposal.id === latestSpecEditProposalId
                                                            const proposalBranch = proposal.git_branch ?? activeProjectGitMetadata.branch
                                                            const proposalCommit = proposal.git_commit ?? activeProjectGitMetadata.commit
                                                            return (
                                                                <li
                                                                    key={key}
                                                                    data-testid={isLatestProposal ? "project-spec-edit-proposal-history-row" : undefined}
                                                                    className="flex justify-start"
                                                                >
                                                                    <div
                                                                        data-testid={isLatestProposal ? "project-spec-edit-proposal-preview" : undefined}
                                                                        className="w-full rounded-md border border-amber-500/30 bg-amber-500/[0.06] px-4 py-3"
                                                                    >
                                                                        <div className="flex flex-wrap items-start justify-between gap-3">
                                                                            <div className="space-y-1">
                                                                                <div className="flex flex-wrap items-center gap-2">
                                                                                    <p className="text-[10px] font-semibold uppercase tracking-wide text-amber-800">
                                                                                        Spec edit card
                                                                                    </p>
                                                                                    <span className={getSurfaceToneClassName(statusPresentation.tone)}>
                                                                                        {statusPresentation.label}
                                                                                    </span>
                                                                                </div>
                                                                                <p className="text-sm font-medium text-foreground">{proposal.summary}</p>
                                                                            </div>
                                                                            <p className="text-[11px] text-muted-foreground">
                                                                                {proposal.changes.length} changed section{proposal.changes.length === 1 ? "" : "s"}
                                                                            </p>
                                                                        </div>
                                                                        <div className="mt-3 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
                                                                            <span>{formatConversationTimestamp(proposal.created_at)}</span>
                                                                            <span className="font-mono">{proposal.id}</span>
                                                                        </div>
                                                                        <ul className="mt-3 space-y-2">
                                                                            {proposal.changes.map((change, changeIndex) => {
                                                                                const diffLines = buildProposalDiffLines(change)
                                                                                const shouldCollapse = diffLines.length > PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT
                                                                                const changeKey = buildProposalChangeKey(proposal.id, change.path, changeIndex)
                                                                                const isExpanded = expandedProposalChanges[changeKey] === true
                                                                                const visibleLines = shouldCollapse && !isExpanded
                                                                                    ? diffLines.slice(0, PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT)
                                                                                    : diffLines
                                                                                return (
                                                                                    <li
                                                                                        key={`${proposal.id}-${change.path}-${changeIndex}`}
                                                                                        className="rounded border border-amber-500/20 bg-background/80"
                                                                                    >
                                                                                        <div className="flex items-center justify-between gap-2 border-b border-amber-500/20 px-3 py-2">
                                                                                            <p className="truncate text-[11px] font-medium text-foreground">{change.path}</p>
                                                                                            {shouldCollapse ? (
                                                                                                <button
                                                                                                    type="button"
                                                                                                    onClick={() => toggleProposalChangeExpanded(changeKey)}
                                                                                                    className="inline-flex items-center gap-1 rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                                                                >
                                                                                                    {isExpanded ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
                                                                                                    {isExpanded ? "Collapse" : `Show all (${diffLines.length})`}
                                                                                                </button>
                                                                                            ) : null}
                                                                                        </div>
                                                                                        <div className="space-y-1 px-3 py-3">
                                                                                            {visibleLines.map((line, lineIndex) => (
                                                                                                <p
                                                                                                    key={`${change.path}-${lineIndex}`}
                                                                                                    className={`whitespace-pre-wrap rounded px-1.5 py-0.5 font-mono text-[11px] ${line.type === "removed"
                                                                                                        ? "bg-red-500/10 text-red-800"
                                                                                                        : "bg-emerald-500/10 text-emerald-800"
                                                                                                        }`}
                                                                                                >
                                                                                                    {line.type === "removed" ? "- " : "+ "}
                                                                                                    {line.text}
                                                                                                </p>
                                                                                            ))}
                                                                                            {shouldCollapse && !isExpanded ? (
                                                                                                <p className="text-[10px] text-muted-foreground">
                                                                                                    Showing first {PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT} of {diffLines.length} lines.
                                                                                                </p>
                                                                                            ) : null}
                                                                                        </div>
                                                                                    </li>
                                                                                )
                                                                            })}
                                                                        </ul>
                                                                        {proposal.status === "pending" ? (
                                                                            <div className="mt-3 flex flex-wrap items-center gap-2">
                                                                                <button
                                                                                    data-testid={isLatestProposal ? "project-spec-edit-proposal-apply-button" : undefined}
                                                                                    type="button"
                                                                                    onClick={() => {
                                                                                        void onApproveSpecEditProposal(proposal)
                                                                                    }}
                                                                                    disabled={pendingSpecProposalId === proposal.id}
                                                                                    className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                                                >
                                                                                    Apply proposal
                                                                                </button>
                                                                                <button
                                                                                    data-testid={isLatestProposal ? "project-spec-edit-proposal-reject-button" : undefined}
                                                                                    type="button"
                                                                                    onClick={() => {
                                                                                        void onRejectSpecEditProposal(proposal)
                                                                                    }}
                                                                                    disabled={pendingSpecProposalId === proposal.id}
                                                                                    className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                                                >
                                                                                    Reject proposal
                                                                                </button>
                                                                            </div>
                                                                        ) : proposal.status === "applied" ? (
                                                                            <div className="mt-3 space-y-1 text-[11px] text-muted-foreground">
                                                                                <p>
                                                                                    Canonical spec edit:{" "}
                                                                                    <span className="font-mono text-foreground">
                                                                                        {proposal.canonical_spec_edit_id || "Pending canonical ID"}
                                                                                    </span>
                                                                                </p>
                                                                                {(proposalBranch || proposalCommit) ? (
                                                                                    <p>
                                                                                        Git anchor:{" "}
                                                                                        <span className="font-mono text-foreground">
                                                                                            {proposalBranch || "detached"}@{proposalCommit || "unknown"}
                                                                                        </span>
                                                                                    </p>
                                                                                ) : null}
                                                                            </div>
                                                                        ) : (
                                                                            <p className="mt-3 text-[11px] text-muted-foreground">
                                                                                This spec edit was rejected. Draft a follow-up change in chat if you want to replace it.
                                                                            </p>
                                                                        )}
                                                                    </div>
                                                                </li>
                                                            )
                                                        }

                                                        if (entry.kind === "execution_card") {
                                                            const executionCard = activeExecutionCardsById.get(entry.artifactId) || null
                                                            if (!executionCard) {
                                                                return (
                                                                    <li key={key} className="flex justify-start">
                                                                        <div className="w-full rounded-md border border-border bg-muted/40 px-4 py-3 text-xs text-muted-foreground">
                                                                            Execution card artifact unavailable. Refresh the project chat to reload it.
                                                                        </div>
                                                                    </li>
                                                                )
                                                            }
                                                            const statusPresentation = getExecutionCardStatusPresentation(executionCard.status)
                                                            const isLatestExecutionCard = executionCard.id === latestExecutionCardId
                                                            const canReview = executionCard.status === "draft"
                                                            return (
                                                                <li
                                                                    key={key}
                                                                    data-testid={isLatestExecutionCard ? "project-plan-generation-history-row" : undefined}
                                                                    className="flex justify-start"
                                                                >
                                                                    <div
                                                                        data-testid={isLatestExecutionCard ? "project-plan-generation-surface" : undefined}
                                                                        className="w-full rounded-md border border-sky-500/20 bg-sky-500/[0.05] px-4 py-3"
                                                                    >
                                                                        <div className="flex flex-wrap items-start justify-between gap-3">
                                                                            <div className="space-y-1">
                                                                                <div className="flex flex-wrap items-center gap-2">
                                                                                    <p className="text-[10px] font-semibold uppercase tracking-wide text-sky-700">
                                                                                        Execution card
                                                                                    </p>
                                                                                    <span className={getSurfaceToneClassName(statusPresentation.tone)}>
                                                                                        {statusPresentation.label}
                                                                                    </span>
                                                                                </div>
                                                                                <p className="text-sm font-semibold text-foreground">{executionCard.title}</p>
                                                                            </div>
                                                                            <div className="space-y-1 text-right text-[11px] text-muted-foreground">
                                                                                <p className="font-mono text-foreground">{executionCard.id}</p>
                                                                                <p>Updated {formatConversationTimestamp(executionCard.updated_at)}</p>
                                                                            </div>
                                                                        </div>
                                                                        <div className="mt-4 space-y-4">
                                                                            <div className="space-y-2">
                                                                                <p className="text-sm text-foreground">{executionCard.summary}</p>
                                                                                <p className="text-xs leading-5 text-muted-foreground">{executionCard.objective}</p>
                                                                            </div>
                                                                            <section className="space-y-2">
                                                                                <div className="flex items-center justify-between gap-2">
                                                                                    <p className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                                                                        Derived work items
                                                                                    </p>
                                                                                    <p className="text-[11px] text-muted-foreground">
                                                                                        Review this package as a group before dispatch.
                                                                                    </p>
                                                                                </div>
                                                                                <ol className="space-y-2">
                                                                                    {executionCard.work_items.map((item) => (
                                                                                        <li key={item.id} className="rounded-md border border-border bg-background/80 px-3 py-2">
                                                                                            <div className="space-y-1">
                                                                                                <div className="flex flex-wrap items-center gap-2">
                                                                                                    <span className="font-mono text-[10px] text-muted-foreground">{item.id}</span>
                                                                                                    <p className="text-xs font-medium text-foreground">{item.title}</p>
                                                                                                </div>
                                                                                                <p className="text-[11px] leading-5 text-muted-foreground">{item.description}</p>
                                                                                                {item.acceptance_criteria.length > 0 ? (
                                                                                                    <ul className="space-y-1 pt-1">
                                                                                                        {item.acceptance_criteria.map((criterion, criterionIndex) => (
                                                                                                            <li key={`${item.id}-criterion-${criterionIndex}`} className="text-[11px] text-muted-foreground">
                                                                                                                - {criterion}
                                                                                                            </li>
                                                                                                        ))}
                                                                                                    </ul>
                                                                                                ) : null}
                                                                                            </div>
                                                                                        </li>
                                                                                    ))}
                                                                                </ol>
                                                                            </section>
                                                                            <section
                                                                                data-testid={isLatestExecutionCard ? "project-plan-gate-surface" : undefined}
                                                                                className="space-y-2 rounded-md border border-border bg-background/80 px-3 py-3"
                                                                            >
                                                                                <div className="flex flex-wrap items-center justify-between gap-2">
                                                                                    <div>
                                                                                        <p className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                                                                            Review decision
                                                                                        </p>
                                                                                        <p className="text-xs text-foreground">
                                                                                            Execution card status: <span className="font-medium">{executionCard.status}</span>
                                                                                        </p>
                                                                                    </div>
                                                                                </div>
                                                                                {canReview ? (
                                                                                    <div className="flex flex-wrap items-center gap-2">
                                                                                        <button
                                                                                            data-testid={isLatestExecutionCard ? "project-plan-approve-button" : undefined}
                                                                                            type="button"
                                                                                            onClick={() => {
                                                                                                void onReviewExecutionCard(executionCard, "approved")
                                                                                            }}
                                                                                            disabled={pendingExecutionCardId === executionCard.id}
                                                                                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                                                        >
                                                                                            Approve plan
                                                                                        </button>
                                                                                        <button
                                                                                            data-testid={isLatestExecutionCard ? "project-plan-reject-button" : undefined}
                                                                                            type="button"
                                                                                            onClick={() => {
                                                                                                void onReviewExecutionCard(executionCard, "rejected")
                                                                                            }}
                                                                                            disabled={pendingExecutionCardId === executionCard.id}
                                                                                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                                                        >
                                                                                            Reject plan
                                                                                        </button>
                                                                                        <button
                                                                                            data-testid={isLatestExecutionCard ? "project-plan-request-revision-button" : undefined}
                                                                                            type="button"
                                                                                            onClick={() => {
                                                                                                void onReviewExecutionCard(executionCard, "revision_requested")
                                                                                            }}
                                                                                            disabled={pendingExecutionCardId === executionCard.id}
                                                                                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                                                        >
                                                                                            Request revision
                                                                                        </button>
                                                                                    </div>
                                                                                ) : null}
                                                                            </section>
                                                                            <div className="flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
                                                                                <span className="font-mono text-foreground">{executionCard.source_spec_edit_id}</span>
                                                                                <span>/</span>
                                                                                <span className="font-mono text-foreground">{executionCard.source_workflow_run_id}</span>
                                                                                {executionCard.flow_source ? (
                                                                                    <>
                                                                                        <span>/</span>
                                                                                        <span className="font-mono text-foreground">{executionCard.flow_source}</span>
                                                                                    </>
                                                                                ) : null}
                                                                            </div>
                                                                        </div>
                                                                    </div>
                                                                </li>
                                                            )
                                                        }

                                                        return (
                                                            <li
                                                                key={key}
                                                                className={`flex ${entry.role === "user" ? "justify-end" : "justify-start"}`}
                                                            >
                                                                <div
                                                                    className={`max-w-[85%] rounded border px-3 py-2 ${entry.role === "user"
                                                                        ? "border-primary/40 bg-primary/10 text-foreground"
                                                                        : entry.presentation === "thinking"
                                                                            ? "border-border/80 bg-background text-muted-foreground"
                                                                            : "border-border bg-muted/40 text-foreground"
                                                                        }`}
                                                                >
                                                                    <p className="text-[10px] font-semibold uppercase tracking-wide opacity-70">
                                                                        {entry.role === "assistant"
                                                                            ? (entry.presentation === "thinking" ? "Thinking" : "Spark Spawn")
                                                                            : entry.role}
                                                                    </p>
                                                                    <p className={`whitespace-pre-wrap text-xs leading-5 ${entry.presentation === "thinking" ? "italic" : ""}`}>
                                                                        {entry.role === "assistant" && entry.status !== "complete" && !entry.content.trim()
                                                                            ? entry.status === "failed"
                                                                                ? (entry.error || "Response failed.")
                                                                                : "Thinking..."
                                                                            : entry.content}
                                                                    </p>
                                                                    <p className="mt-1 text-[10px] opacity-70">{formatConversationTimestamp(entry.timestamp)}</p>
                                                                </div>
                                                            </li>
                                                        )
                                                    })}
                                                </ol>
                                            )}
                                        </div>
                                    </div>
                                    {!isConversationPinnedToBottom && hasRenderableConversationHistory ? (
                                        <div className="flex justify-end">
                                            <button
                                                type="button"
                                                data-testid="project-ai-conversation-jump-to-bottom"
                                                onClick={scrollConversationToBottom}
                                                className="rounded border border-border bg-background/90 px-2 py-1 text-[11px] text-muted-foreground shadow-sm hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            >
                                                Jump to bottom
                                            </button>
                                        </div>
                                    ) : null}
                                    <form
                                        data-testid="project-ai-conversation-composer"
                                        onSubmit={onChatComposerSubmit}
                                        className="shrink-0 space-y-2 pt-1"
                                    >
                                        <textarea
                                            id="project-ai-conversation-input"
                                            data-testid="project-ai-conversation-input"
                                            value={chatDraft}
                                            onChange={(event) => setChatDraft(event.target.value)}
                                            onKeyDown={onChatComposerKeyDown}
                                            aria-label="Message"
                                            placeholder="Describe the spec change or requirement you want to work on..."
                                            rows={4}
                                            className="w-full rounded border border-border bg-background px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        />
                                        <div className="flex items-center justify-between gap-2">
                                            <p className="text-[11px] text-muted-foreground">
                                                Press Enter to send. Use Shift+Enter for a new line.
                                            </p>
                                            <button
                                                data-testid="project-ai-conversation-send-button"
                                                type="submit"
                                                disabled={chatDraft.trim().length === 0 || isSendingChat}
                                                className="rounded border border-border px-3 py-1.5 text-xs font-medium hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                            >
                                                {chatSendButtonLabel}
                                            </button>
                                        </div>
                                        {panelError ? (
                                            <p className="text-[11px] text-destructive">{panelError}</p>
                                        ) : null}
                                    </form>
                                </div>
                            )}
                        </div>
                    </HomeWorkspace>
                </div>
            </div>
        </section>
    )
}

export const ProjectsPanel = HomePanel
