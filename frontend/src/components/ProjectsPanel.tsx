import { type PlanStatus, type ProjectRegistrationResult, useStore } from "@/store"
import { type ChangeEvent, type FormEvent, type KeyboardEvent, type PointerEvent as ReactPointerEvent, useEffect, useMemo, useRef, useState } from "react"
import {
    ApiHttpError,
    type ConversationSegmentResponse,
    type ConversationSegmentUpsertEventResponse,
    type ConversationSummaryResponse,
    type ConversationSnapshotResponse,
    type ConversationTurnUpsertEventResponse,
    type ConversationTurnResponse,
    deleteConversationValidated,
    deleteProjectValidated,
    type ExecutionCardResponse,
    type FlowRunRequestResponse,
    type SpecEditProposalResponse,
    approveSpecEditProposalValidated,
    conversationEventsUrl,
    fetchConversationSnapshotValidated,
    fetchProjectRegistryValidated,
    fetchProjectConversationListValidated,
    fetchProjectMetadataValidated,
    pickProjectDirectoryValidated,
    parseConversationSnapshotResponse,
    parseConversationStreamEventResponse,
    rejectSpecEditProposalValidated,
    registerProjectValidated,
    reviewExecutionCardValidated,
    reviewFlowRunRequestValidated,
    sendConversationTurnValidated,
    updateProjectStateValidated,
} from "@/lib/workspaceClient"
import { useNarrowViewport } from "@/lib/useNarrowViewport"
import { isAbsoluteProjectPath, normalizeProjectPath } from "@/lib/projectPaths"
import { ProjectConversationHistory } from "@/components/projects/ProjectConversationHistory"
import { ProjectConversationSurface } from "@/components/projects/ProjectConversationSurface"
import { ProjectsSidebar } from "@/components/projects/ProjectsSidebar"
import type { ConversationTimelineEntry } from "@/components/projects/types"
import type { ProjectGitMetadata } from "@/components/projects/presentation"

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

const DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT = 320
const HOME_SIDEBAR_MIN_PRIMARY_HEIGHT = 208
const HOME_SIDEBAR_MIN_SECONDARY_HEIGHT = 208
const HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT = 12
const CONVERSATION_BOTTOM_THRESHOLD_PX = 24

type PickerFileWithPath = File & {
    path?: string
    webkitRelativePath?: string
}

const EMPTY_PROJECT_GIT_METADATA: ProjectGitMetadata = {
    branch: null,
    commit: null,
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
    flow_bindings?: Record<string, string>
}) => ({
    directoryPath: project.project_path,
    isFavorite: project.is_favorite === true,
    lastAccessedAt: typeof project.last_accessed_at === "string" ? project.last_accessed_at : null,
    activeConversationId: typeof project.active_conversation_id === "string" ? project.active_conversation_id : null,
    flowBindings: project.flow_bindings ?? {},
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

type OptimisticSendState = {
    conversationId: string
    message: string
    createdAt: string
}

const ensureConversationSnapshotShell = (
    conversationId: string,
    projectPath: string,
    title = "New thread",
): ConversationSnapshotResponse => ({
    schema_version: 4,
    conversation_id: conversationId,
    conversation_handle: "",
    project_path: projectPath,
    title,
    created_at: "",
    updated_at: "",
    turns: [],
    segments: [],
    event_log: [],
    spec_edit_proposals: [],
    flow_run_requests: [],
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

const upsertConversationSegment = (
    snapshot: ConversationSnapshotResponse,
    segment: ConversationSegmentResponse,
): ConversationSnapshotResponse => {
    const nextSegments = [...snapshot.segments]
    const existingIndex = nextSegments.findIndex((entry) => entry.id === segment.id)
    if (existingIndex >= 0) {
        nextSegments[existingIndex] = segment
    } else {
        nextSegments.push(segment)
    }
    nextSegments.sort((left, right) => {
        if (left.turn_id === right.turn_id) {
            const orderDelta = left.order - right.order
            if (orderDelta !== 0) {
                return orderDelta
            }
            const timestampDelta = left.timestamp.localeCompare(right.timestamp)
            if (timestampDelta !== 0) {
                return timestampDelta
            }
            return left.id.localeCompare(right.id)
        }
        return left.timestamp.localeCompare(right.timestamp)
    })
    return {
        ...snapshot,
        segments: nextSegments,
    }
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
    if (incomingTurn.content.trim().length > 0) {
        return incomingTurn
    }
    return {
        ...incomingTurn,
        content: currentTurn?.content ?? '',
    }
}

const scoreConversationSnapshotFreshness = (snapshot: ConversationSnapshotResponse): number => {
    const turnStatusScore = snapshot.turns.reduce((score, turn) => {
        if (turn.status === 'failed') {
            return score + 4
        }
        if (turn.status === 'complete') {
            return score + 3
        }
        if (turn.status === 'streaming') {
            return score + 2
        }
        return score + 1
    }, 0)
    const contentScore = snapshot.turns.reduce((score, turn) => score + turn.content.length, 0)
    return (
        snapshot.turns.length * 100000
        + snapshot.segments.length * 1000
        + turnStatusScore * 100
        + contentScore
    )
}

const compareConversationSnapshotFreshness = (
    left: ConversationSnapshotResponse,
    right: ConversationSnapshotResponse,
): number => {
    const updatedAtCompare = left.updated_at.localeCompare(right.updated_at)
    if (updatedAtCompare !== 0) {
        return updatedAtCompare
    }
    return scoreConversationSnapshotFreshness(left) - scoreConversationSnapshotFreshness(right)
}

const formatWorkedDuration = (elapsedSeconds: number): string => {
    if (elapsedSeconds < 60) {
        return `${elapsedSeconds}s`
    }
    if (elapsedSeconds < 3600) {
        const minutes = Math.floor(elapsedSeconds / 60)
        const seconds = elapsedSeconds % 60
        return seconds === 0 ? `${minutes}m` : `${minutes}m ${seconds}s`
    }
    const hours = Math.floor(elapsedSeconds / 3600)
    const minutes = Math.floor((elapsedSeconds % 3600) / 60)
    return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`
}

const resolveWorkedElapsedSeconds = (
    turn: ConversationTurnResponse,
    turnSegments: ConversationSegmentResponse[],
    completedTimestamp: string,
): number | null => {
    const completedMs = Date.parse(completedTimestamp)
    if (Number.isNaN(completedMs)) {
        return null
    }
    const candidateTimestamps = [turn.timestamp, ...turnSegments.map((segment) => segment.timestamp)]
        .map((value) => Date.parse(value))
        .filter((value) => !Number.isNaN(value) && value <= completedMs)
    if (candidateTimestamps.length === 0) {
        return null
    }
    const startedMs = Math.min(...candidateTimestamps)
    return Math.max(0, Math.round((completedMs - startedMs) / 1000))
}

const buildAssistantTimelineEntries = (
    turn: ConversationTurnResponse,
    turnSegments: ConversationSegmentResponse[],
): ConversationTimelineEntry[] => {
    const entries: ConversationTimelineEntry[] = []
    let hadWorkActivity = false
    let insertedFinalSeparator = false
    const sortedSegments = [...turnSegments].sort((left, right) => left.order - right.order)

    sortedSegments.forEach((segment) => {
        if (!insertedFinalSeparator && hadWorkActivity && segment.kind === "assistant_message") {
            const elapsedSeconds = resolveWorkedElapsedSeconds(turn, turnSegments, segment.timestamp)
            const label = elapsedSeconds === null
                ? "Worked"
                : `Worked for ${formatWorkedDuration(elapsedSeconds)}`
            entries.push({
                id: `${turn.id}:final-separator:${entries.length}`,
                kind: "final_separator",
                role: "system",
                timestamp: segment.timestamp,
                label,
            })
            insertedFinalSeparator = true
        }
        if (segment.kind === "assistant_message") {
            entries.push({
                id: segment.id,
                kind: "message",
                role: "assistant",
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === "running" ? "streaming" : segment.status,
                error: segment.error ?? null,
            })
            return
        }
        if (segment.kind === "reasoning") {
            entries.push({
                id: segment.id,
                kind: "message",
                role: "assistant",
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === "running" ? "streaming" : segment.status,
                error: segment.error ?? null,
                presentation: "thinking",
            })
            return
        }
        if (segment.kind === "tool_call" && segment.tool_call) {
            entries.push({
                id: segment.id,
                kind: "tool_call",
                role: "system",
                timestamp: segment.timestamp,
                toolCall: {
                    id: segment.tool_call.id,
                    kind: segment.tool_call.kind,
                    status: segment.tool_call.status,
                    title: segment.tool_call.title,
                    command: segment.tool_call.command ?? null,
                    output: segment.tool_call.output ?? null,
                    filePaths: segment.tool_call.file_paths,
                },
            })
            hadWorkActivity = true
            return
        }
        if (segment.kind === "spec_edit_proposal" && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: "spec_edit_proposal",
                role: "system",
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
            return
        }
        if (segment.kind === "flow_run_request" && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: "flow_run_request",
                role: "system",
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
            return
        }
        if (segment.kind === "execution_card" && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: "execution_card",
                role: "system",
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
        }
    })

    if (entries.length === 0) {
        const presentation = turn.status === "complete" || turn.status === "failed" ? "default" : "thinking"
        entries.push({
            id: `${turn.id}:${presentation}:placeholder`,
            kind: "message",
            role: "assistant",
            content: turn.content,
            timestamp: turn.timestamp,
            status: turn.status,
            error: turn.error ?? null,
            presentation,
        })
    }

    return entries
}

const buildConversationTimelineEntries = (
    snapshot: ConversationSnapshotResponse | null,
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

    const timeline: ConversationTimelineEntry[] = []
    const segmentsByTurn = new Map<string, ConversationSegmentResponse[]>()
    snapshot.segments.forEach((segment) => {
        const entries = segmentsByTurn.get(segment.turn_id) || []
        entries.push(segment)
        segmentsByTurn.set(segment.turn_id, entries)
    })
    snapshot.turns.forEach((turn) => {
        if (turn.role === "user" || turn.role === "assistant") {
            if (turn.role === "assistant") {
                timeline.push(...buildAssistantTimelineEntries(turn, segmentsByTurn.get(turn.id) || []))
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

    const optimisticEntries: ConversationTimelineEntry[] = []
    optimisticEntries.push({
        id: `${optimisticSend.conversationId}:optimistic:user`,
        kind: "message",
        role: "user",
        content: optimisticSend.message,
        timestamp: optimisticSend.createdAt,
        status: "complete",
    })
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
    const removeProject = useStore((state) => state.removeProject)
    const setProjectRegistrationError = useStore((state) => state.setProjectRegistrationError)
    const clearProjectRegistrationError = useStore((state) => state.clearProjectRegistrationError)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setConversationId = useStore((state) => state.setConversationId)
    const appendProjectEventEntry = useStore((state) => state.appendProjectEventEntry)
    const updateProjectScopedWorkspace = useStore((state) => state.updateProjectScopedWorkspace)
    const model = useStore((state) => state.model)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)

    const [projectGitMetadata, setProjectGitMetadata] = useState<Record<string, ProjectGitMetadata>>({})
    const [projectConversationSnapshots, setProjectConversationSnapshots] = useState<Record<string, ConversationSnapshotResponse>>({})
    const [projectConversationSummaries, setProjectConversationSummaries] = useState<Record<string, ConversationSummaryResponse[]>>({})
    const [chatDraft, setChatDraft] = useState("")
    const [panelError, setPanelError] = useState<string | null>(null)
    const [optimisticSend, setOptimisticSend] = useState<OptimisticSendState | null>(null)
    const [pendingSpecProposalId, setPendingSpecProposalId] = useState<string | null>(null)
    const [pendingFlowRunRequestId, setPendingFlowRunRequestId] = useState<string | null>(null)
    const [pendingExecutionCardId, setPendingExecutionCardId] = useState<string | null>(null)
    const [pendingDeleteConversationId, setPendingDeleteConversationId] = useState<string | null>(null)
    const [pendingDeleteProjectPath, setPendingDeleteProjectPath] = useState<string | null>(null)
    const [expandedProposalChanges, setExpandedProposalChanges] = useState<Record<string, boolean>>({})
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const [expandedThinkingEntries, setExpandedThinkingEntries] = useState<Record<string, boolean>>({})
    const [homeSidebarPrimaryHeight, setHomeSidebarPrimaryHeight] = useState(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)
    const [isHomeSidebarResizing, setIsHomeSidebarResizing] = useState(false)
    const [isConversationPinnedToBottom, setIsConversationPinnedToBottom] = useState(true)

    const projectDirectoryPickerInputRef = useRef<HTMLInputElement | null>(null)
    const homeSidebarRef = useRef<HTMLDivElement | null>(null)
    const homeSidebarResizeRef = useRef<{ startY: number; startHeight: number } | null>(null)
    const conversationBodyRef = useRef<HTMLDivElement | null>(null)
    const applyConversationSnapshotRef = useRef<((projectPath: string, snapshot: ConversationSnapshotResponse, source?: string) => void) | null>(null)
    const applyConversationStreamEventRef = useRef<((projectPath: string, event: ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse, source?: string) => void) | null>(null)

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null
    const activeProjectLabel = activeProjectPath ? formatProjectListLabel(activeProjectPath) : null
    const activeProjectGitMetadata = activeProjectPath
        ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
        : EMPTY_PROJECT_GIT_METADATA
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const activeConversationSnapshot = activeConversationId ? projectConversationSnapshots[activeConversationId] || null : null
    const activeProjectConversationSummaries = activeProjectPath ? projectConversationSummaries[activeProjectPath] || [] : []
    const activeProjectEventLog = activeProjectScope?.projectEventLog || []
    const activeConversationHistory = useMemo(
        () => buildConversationTimelineEntries(
            activeConversationSnapshot,
            optimisticSend && optimisticSend.conversationId === activeConversationId ? optimisticSend : null,
        ),
        [activeConversationId, activeConversationSnapshot, optimisticSend],
    )
    const activeSpecEditProposals = activeConversationSnapshot?.spec_edit_proposals || []
    const activeFlowRunRequests = activeConversationSnapshot?.flow_run_requests || []
    const activeExecutionCards = activeConversationSnapshot?.execution_cards || []
    const latestSpecEditProposalId = activeSpecEditProposals.length > 0
        ? activeSpecEditProposals[activeSpecEditProposals.length - 1]?.id || null
        : null
    const latestFlowRunRequestId = activeFlowRunRequests.length > 0
        ? activeFlowRunRequests[activeFlowRunRequests.length - 1]?.id || null
        : null
    const latestExecutionCardId = activeExecutionCards.length > 0
        ? activeExecutionCards[activeExecutionCards.length - 1]?.id || null
        : null
    const activeSpecEditProposalsById = new Map(activeSpecEditProposals.map((proposal) => [proposal.id, proposal]))
    const activeFlowRunRequestsById = new Map(activeFlowRunRequests.map((request) => [request.id, request]))
    const activeExecutionCardsById = new Map(activeExecutionCards.map((executionCard) => [executionCard.id, executionCard]))
    const hasRenderableConversationHistory = activeConversationHistory.some((entry) => (
        entry.kind === "spec_edit_proposal"
        || entry.kind === "flow_run_request"
        || entry.kind === "execution_card"
        || entry.kind === "tool_call"
        || entry.role === "user"
        || entry.role === "assistant"
    ))
    const hasActiveAssistantTurn = (activeConversationSnapshot?.turns || []).some((turn) => (
        turn.role === "assistant" && (turn.status === "pending" || turn.status === "streaming")
    ))
    const isChatInputDisabled = hasActiveAssistantTurn
    const chatSendButtonLabel = hasActiveAssistantTurn ? "Thinking..." : "Send"

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
            return projectConversationSummaries[projectPath] || []
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
        let shouldApplySnapshot = true
        setProjectConversationSnapshots((current) => {
            const existingSnapshot = current[snapshot.conversation_id]
            if (existingSnapshot && compareConversationSnapshotFreshness(existingSnapshot, snapshot) >= 0) {
                shouldApplySnapshot = false
                return current
            }
            return current
        })
        if (!shouldApplySnapshot) {
            debugProjectChat("skip stale conversation snapshot", {
                source,
                projectPath,
                conversationId: snapshot.conversation_id,
                snapshotUpdatedAt: snapshot.updated_at,
            })
            return
        }
        const latestProjectScope = useStore.getState().projectScopedWorkspaces[projectPath]
        const shouldSyncActiveWorkspace = options?.forceWorkspaceSync === true
            || latestProjectScope?.conversationId === snapshot.conversation_id
        const latestApprovedProposal = getLatestApprovedSpecEditProposal(snapshot)
        const latestExecutionCard = getLatestExecutionCard(snapshot)
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
        setProjectConversationSummaries((current) => ({
            ...current,
            [projectPath]: upsertConversationSummary(current[projectPath] || [], {
                conversation_id: snapshot.conversation_id,
                conversation_handle: snapshot.conversation_handle,
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
        event: ConversationTurnUpsertEventResponse | ConversationSegmentUpsertEventResponse,
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
            } else {
                mergedSnapshot = {
                    ...existingSnapshot,
                    project_path: event.project_path,
                    title: event.title,
                    updated_at: event.updated_at,
                }
            }
            if (event.type === "segment_upsert") {
                mergedSnapshot = upsertConversationSegment(mergedSnapshot, event.segment)
            }
            nextSnapshot = mergedSnapshot
            return {
                ...current,
                [event.conversation_id]: mergedSnapshot,
            }
        })
        const updatedSnapshot = nextSnapshot as ConversationSnapshotResponse | null
        if (updatedSnapshot === null) {
            return
        }
        setProjectConversationSummaries((current) => ({
            ...current,
            [projectPath]: upsertConversationSummary(current[projectPath] || [], {
                conversation_id: updatedSnapshot.conversation_id,
                conversation_handle: updatedSnapshot.conversation_handle,
                project_path: updatedSnapshot.project_path,
                title: updatedSnapshot.title,
                created_at: updatedSnapshot.created_at,
                updated_at: updatedSnapshot.updated_at,
                last_message_preview: updatedSnapshot.turns
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
                        const metadata = await fetchProjectMetadataValidated(projectPath)
                        return [
                            projectPath,
                            {
                                branch: asProjectGitMetadataField(metadata.branch),
                                commit: asProjectGitMetadataField(metadata.commit),
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
        setExpandedToolCalls({})
    }, [activeConversationId, activeProjectPath])

    useEffect(() => {
        setExpandedThinkingEntries({})
    }, [activeConversationId, activeProjectPath])

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
            const eventStreamUrl = conversationEventsUrl(activeConversationId, activeProjectPath)
            eventSource = new EventSource(eventStreamUrl)
            eventSource.onmessage = (event) => {
                if (isCancelled) {
                    return
                }
                try {
                    const payload = JSON.parse(event.data) as { type?: string; state?: unknown }
                    if (payload.type === "conversation_snapshot") {
                        const snapshot = parseConversationSnapshotResponse(payload.state, "/workspace/api/conversations/{id}/events")
                        applyConversationSnapshotRef.current?.(activeProjectPath, snapshot, "event-stream-snapshot")
                        return
                    }
                    const parsedEvent = parseConversationStreamEventResponse(payload, "/workspace/api/conversations/{id}/events")
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
            const payload = await fetchProjectMetadataValidated(projectPath)
            return {
                metadata: {
                    branch: asProjectGitMetadataField(payload.branch),
                    commit: asProjectGitMetadataField(payload.commit),
                },
            }
        } catch (err) {
            let message = "Unable to verify project Git state."
            if (err instanceof ApiHttpError && err.detail) {
                message = err.detail
            }
            return { metadata: { ...EMPTY_PROJECT_GIT_METADATA }, error: message }
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
        const normalizedProjectPath = validation.normalizedPath
        const gitMetadata = await ensureProjectGitRepository(normalizedProjectPath)
        if (!gitMetadata) {
            return
        }
        const result = registerProject(normalizedProjectPath)
        if (!result.ok) {
            setProjectRegistrationError(result.error ?? "Unable to register the project.")
            return
        }
        try {
            const projectRecord = await registerProjectValidated(normalizedProjectPath)
            upsertProjectRegistryEntry(toHydratedProjectRecord(projectRecord))
        } catch (error) {
            useStore.setState((state) => {
                const nextProjectRegistry = { ...state.projectRegistry }
                const nextProjectScopedWorkspaces = { ...state.projectScopedWorkspaces }
                delete nextProjectRegistry[normalizedProjectPath]
                delete nextProjectScopedWorkspaces[normalizedProjectPath]
                const nextActiveProjectPath = state.activeProjectPath === normalizedProjectPath ? null : state.activeProjectPath
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
                conversation_handle: "",
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

    const onDeleteProject = async (projectPath: string) => {
        const projectLabel = formatProjectListLabel(projectPath)
        if (
            typeof window !== "undefined"
            && !window.confirm(
                `Remove project "${projectLabel}" from Spark Spawn? This deletes its local threads, workflow history, and runs, but does not delete the project files.`,
            )
        ) {
            return
        }

        setPanelError(null)
        setPendingDeleteProjectPath(projectPath)
        try {
            await deleteProjectValidated(projectPath)

            setProjectGitMetadata((current) => {
                const next = { ...current }
                delete next[projectPath]
                return next
            })
            setProjectConversationSummaries((current) => {
                const next = { ...current }
                delete next[projectPath]
                return next
            })
            setProjectConversationSnapshots((current) => {
                const next: Record<string, ConversationSnapshotResponse> = {}
                Object.entries(current).forEach(([conversationId, snapshot]) => {
                    if (snapshot.project_path !== projectPath) {
                        next[conversationId] = snapshot
                    }
                })
                return next
            })

            const fallbackProjectPath = activeProjectPath === projectPath
                ? orderedProjects.find((project) => project.directoryPath !== projectPath)?.directoryPath || null
                : null
            removeProject(projectPath, fallbackProjectPath)
        } catch (error) {
            const message = extractApiErrorMessage(error, "Unable to remove the project.")
            setPanelError(message)
            appendLocalProjectEvent(`Project removal failed: ${message}`)
        } finally {
            setPendingDeleteProjectPath(null)
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
        if (isChatInputDisabled) {
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

        setPanelError(null)
        setChatDraft("")
        setOptimisticSend({
            conversationId,
            message: trimmed,
            createdAt: optimisticCreatedAt,
        })
        try {
            const snapshot = await sendConversationTurnValidated(conversationId, {
                project_path: activeProjectPath,
                message: trimmed,
                model: model.trim() || null,
            })
            setOptimisticSend(null)
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

    const onReviewFlowRunRequest = async (
        flowRunRequest: FlowRunRequestResponse,
        disposition: "approved" | "rejected",
    ) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        const reviewMessage = disposition === "approved"
            ? "Approved for launch."
            : window.prompt(
                "Describe why this flow run request should be rejected.",
                "",
            )?.trim() || ""

        if (!reviewMessage) {
            return
        }

        setPendingFlowRunRequestId(flowRunRequest.id)
        setPanelError(null)
        try {
            const snapshot = await reviewFlowRunRequestValidated(activeConversationId, flowRunRequest.id, {
                project_path: activeProjectPath,
                disposition,
                message: reviewMessage,
                model: model.trim() || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, "flow-run-request-review")
        } catch (error) {
            const message = extractApiErrorMessage(error, "Unable to review the flow run request.")
            setPanelError(message)
            appendLocalProjectEvent(`Flow run request review failed: ${message}`)
        } finally {
            setPendingFlowRunRequestId(null)
        }
    }

    const onOpenFlowRun = (flowRunRequest: FlowRunRequestResponse) => {
        if (!flowRunRequest.run_id) {
            return
        }
        setSelectedRunId(flowRunRequest.run_id)
        setExecutionFlow(flowRunRequest.flow_name || null)
        setViewMode("execution")
    }

    const formatConversationTimestamp = (value: string) => {
        const parsed = new Date(value)
        if (Number.isNaN(parsed.getTime())) {
            return value
        }
        return parsed.toLocaleString()
    }

    const toggleProposalChangeExpanded = (changeKey: string) => {
        setExpandedProposalChanges((current) => ({
            ...current,
            [changeKey]: !current[changeKey],
        }))
    }

    const toggleToolCallExpanded = (toolCallId: string) => {
        setExpandedToolCalls((current) => ({
            ...current,
            [toolCallId]: !current[toolCallId],
        }))
    }

    const toggleThinkingEntryExpanded = (entryId: string) => {
        setExpandedThinkingEntries((current) => ({
            ...current,
            [entryId]: !current[entryId],
        }))
    }

    const conversationHistoryContent = (
        <ProjectConversationHistory
            activeConversationId={activeConversationId}
            hasRenderableConversationHistory={hasRenderableConversationHistory}
            activeConversationHistory={activeConversationHistory}
            activeSpecEditProposalsById={activeSpecEditProposalsById}
            activeFlowRunRequestsById={activeFlowRunRequestsById}
            activeExecutionCardsById={activeExecutionCardsById}
            latestSpecEditProposalId={latestSpecEditProposalId}
            latestFlowRunRequestId={latestFlowRunRequestId}
            latestExecutionCardId={latestExecutionCardId}
            activeProjectGitMetadata={activeProjectGitMetadata}
            expandedToolCalls={expandedToolCalls}
            expandedThinkingEntries={expandedThinkingEntries}
            expandedProposalChanges={expandedProposalChanges}
            pendingSpecProposalId={pendingSpecProposalId}
            pendingFlowRunRequestId={pendingFlowRunRequestId}
            pendingExecutionCardId={pendingExecutionCardId}
            formatConversationTimestamp={formatConversationTimestamp}
            onToggleToolCallExpanded={toggleToolCallExpanded}
            onToggleThinkingEntryExpanded={toggleThinkingEntryExpanded}
            onToggleProposalChangeExpanded={toggleProposalChangeExpanded}
            onApproveSpecEditProposal={onApproveSpecEditProposal}
            onRejectSpecEditProposal={onRejectSpecEditProposal}
            onReviewFlowRunRequest={onReviewFlowRunRequest}
            onOpenFlowRun={onOpenFlowRun}
            onReviewExecutionCard={onReviewExecutionCard}
        />
    )

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
                    <ProjectsSidebar
                        isNarrowViewport={isNarrowViewport}
                        homeSidebarRef={homeSidebarRef}
                        homeSidebarPrimaryHeight={homeSidebarPrimaryHeight}
                        projectDirectoryPickerInputRef={projectDirectoryPickerInputRef}
                        projectRegistrationError={projectRegistrationError}
                        orderedProjects={orderedProjects}
                        activeProjectPath={activeProjectPath}
                        activeConversationId={activeConversationId}
                        activeProjectConversationSummaries={activeProjectConversationSummaries}
                        pendingDeleteProjectPath={pendingDeleteProjectPath}
                        pendingDeleteConversationId={pendingDeleteConversationId}
                        activeProjectEventLog={activeProjectEventLog}
                        isHomeSidebarResizing={isHomeSidebarResizing}
                        onOpenProjectDirectoryChooser={onOpenProjectDirectoryChooser}
                        onProjectDirectorySelected={onProjectDirectorySelected}
                        onActivateProject={onActivateProject}
                        onDeleteProject={onDeleteProject}
                        onCreateConversationThread={onCreateConversationThread}
                        onSelectConversationThread={onSelectConversationThread}
                        onDeleteConversationThread={onDeleteConversationThread}
                        onHomeSidebarResizePointerDown={onHomeSidebarResizePointerDown}
                        onHomeSidebarResizeKeyDown={onHomeSidebarResizeKeyDown}
                        formatProjectListLabel={formatProjectListLabel}
                        formatConversationAgeShort={formatConversationAgeShort}
                        formatConversationTimestamp={formatConversationTimestamp}
                    />
                    <ProjectConversationSurface
                        activeProjectLabel={activeProjectLabel}
                        activeProjectPath={activeProjectPath}
                        hasRenderableConversationHistory={hasRenderableConversationHistory}
                        isConversationPinnedToBottom={isConversationPinnedToBottom}
                        isNarrowViewport={isNarrowViewport}
                        chatDraft={chatDraft}
                        chatSendButtonLabel={chatSendButtonLabel}
                        isChatInputDisabled={isChatInputDisabled}
                        panelError={panelError}
                        conversationBodyRef={conversationBodyRef}
                        historyContent={conversationHistoryContent}
                        onSyncConversationPinnedState={syncConversationPinnedState}
                        onScrollConversationToBottom={scrollConversationToBottom}
                        onChatComposerSubmit={onChatComposerSubmit}
                        onChatComposerKeyDown={onChatComposerKeyDown}
                        onChatDraftChange={setChatDraft}
                    />
                </div>
            </div>
        </section>
    )
}

export const ProjectsPanel = HomePanel
