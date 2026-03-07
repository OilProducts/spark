import { type ConversationHistoryEntry, type PlanStatus, type ProjectRegistrationResult, useStore } from "@/store"
import { type ChangeEvent, type FormEvent, type KeyboardEvent, type PointerEvent as ReactPointerEvent, useEffect, useRef, useState } from "react"
import { ChevronDown, ChevronUp } from "lucide-react"
import {
    ApiHttpError,
    type ConversationSummaryResponse,
    type ConversationSnapshotResponse,
    type ExecutionCardResponse,
    type SpecEditProposalResponse,
    approveSpecEditProposalValidated,
    fetchConversationSnapshotValidated,
    fetchProjectConversationListValidated,
    parseConversationSnapshotResponse,
    rejectSpecEditProposalValidated,
    reviewExecutionCardValidated,
    sendConversationTurnValidated,
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

const buildConversationHistoryEntries = (snapshot: ConversationSnapshotResponse): ConversationHistoryEntry[] => (
    snapshot.turns.map((turn) => ({
        role: turn.role,
        content: turn.content,
        timestamp: turn.timestamp,
        kind: turn.kind,
        artifactId: turn.artifact_id ?? null,
        toolCall: turn.tool_call
            ? {
                kind: turn.tool_call.kind,
                status: turn.tool_call.status,
                title: turn.tool_call.title,
                command: turn.tool_call.command ?? null,
                output: turn.tool_call.output ?? null,
                filePaths: turn.tool_call.file_paths,
            }
            : null,
    }))
)

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

const hasAgentActivityAfterLatestUserTurn = (entries: ConversationHistoryEntry[]) => {
    let latestUserTurnIndex = -1
    entries.forEach((entry, index) => {
        if (entry.role === "user") {
            latestUserTurnIndex = index
        }
    })

    if (latestUserTurnIndex === -1) {
        return false
    }

    return entries.slice(latestUserTurnIndex + 1).some((entry) => (
        entry.kind === "tool_call"
        || entry.kind === "spec_edit_proposal"
        || entry.kind === "execution_card"
        || entry.role === "assistant"
    ))
}

export function HomePanel() {
    const projectRegistry = useStore((state) => state.projectRegistry)
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
    const appendConversationHistoryEntry = useStore((state) => state.appendConversationHistoryEntry)
    const appendProjectEventEntry = useStore((state) => state.appendProjectEventEntry)
    const updateProjectScopedWorkspace = useStore((state) => state.updateProjectScopedWorkspace)
    const activeFlow = useStore((state) => state.activeFlow)
    const model = useStore((state) => state.model)

    const [projectGitMetadata, setProjectGitMetadata] = useState<Record<string, ProjectGitMetadata>>({})
    const [projectConversationSnapshots, setProjectConversationSnapshots] = useState<Record<string, ConversationSnapshotResponse>>({})
    const [projectConversationSummaries, setProjectConversationSummaries] = useState<Record<string, ConversationSummaryResponse[]>>({})
    const [chatDraft, setChatDraft] = useState("")
    const [panelError, setPanelError] = useState<string | null>(null)
    const [isSendingChat, setIsSendingChat] = useState(false)
    const [pendingSpecProposalId, setPendingSpecProposalId] = useState<string | null>(null)
    const [pendingExecutionCardId, setPendingExecutionCardId] = useState<string | null>(null)
    const [expandedProposalChanges, setExpandedProposalChanges] = useState<Record<string, boolean>>({})
    const [homeSidebarPrimaryHeight, setHomeSidebarPrimaryHeight] = useState(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)
    const [isHomeSidebarResizing, setIsHomeSidebarResizing] = useState(false)
    const [isConversationPinnedToBottom, setIsConversationPinnedToBottom] = useState(true)

    const projectDirectoryPickerInputRef = useRef<HTMLInputElement | null>(null)
    const homeSidebarRef = useRef<HTMLDivElement | null>(null)
    const homeSidebarResizeRef = useRef<{ startY: number; startHeight: number } | null>(null)
    const conversationBodyRef = useRef<HTMLDivElement | null>(null)
    const applyConversationSnapshotRef = useRef<((projectPath: string, snapshot: ConversationSnapshotResponse) => void) | null>(null)

    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null
    const activeProjectLabel = activeProjectPath ? formatProjectListLabel(activeProjectPath) : null
    const activeProjectGitMetadata = activeProjectPath
        ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
        : EMPTY_PROJECT_GIT_METADATA
    const activeConversationId = activeProjectScope?.conversationId ?? null
    const activeConversationSnapshot = activeConversationId ? projectConversationSnapshots[activeConversationId] || null : null
    const hasLoadedActiveConversationSnapshot = Boolean(activeConversationId && projectConversationSnapshots[activeConversationId])
    const activeProjectConversationSummaries = activeProjectPath ? projectConversationSummaries[activeProjectPath] || [] : []
    const activeConversationHistory = activeProjectScope?.conversationHistory || []
    const activeProjectEventLog = activeProjectScope?.projectEventLog || []
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
    const showPendingAssistantRow = isSendingChat && !hasAgentActivityAfterLatestUserTurn(activeConversationHistory)

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

    const activateConversationThread = (projectPath: string, conversationId: string) => {
        setConversationId(conversationId)
        updateProjectScopedWorkspace(projectPath, {
            conversationId,
            conversationHistory: [],
            specId: null,
            specStatus: "draft",
            specProvenance: null,
            planId: null,
            planStatus: "draft",
            planProvenance: null,
            artifactRunId: null,
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

    const applyConversationSnapshot = (projectPath: string, snapshot: ConversationSnapshotResponse) => {
        const currentScope = projectScopedWorkspaces[projectPath]
        const latestApprovedProposal = getLatestApprovedSpecEditProposal(snapshot)
        const latestExecutionCard = getLatestExecutionCard(snapshot)
        const selectedRunId = snapshot.execution_workflow.run_id
            || latestExecutionCard?.source_workflow_run_id
            || currentScope?.selectedRunId
            || null
        const flowSource = snapshot.execution_workflow.flow_source
            || latestExecutionCard?.flow_source
            || currentScope?.activeFlow
            || null

        setProjectConversationSnapshots((current) => ({
            ...current,
            [snapshot.conversation_id]: snapshot,
        }))
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

        updateProjectScopedWorkspace(projectPath, {
            conversationId: snapshot.conversation_id,
            conversationHistory: buildConversationHistoryEntries(snapshot),
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

    applyConversationSnapshotRef.current = applyConversationSnapshot

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
                activateConversationThread(activeProjectPath, latestConversation.conversation_id)
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
                applyConversationSnapshotRef.current?.(activeProjectPath, snapshot)
            } catch (error) {
                if (isCancelled) {
                    return
                }
                if (error instanceof ApiHttpError && error.status === 404 && !hasLoadedActiveConversationSnapshot) {
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
                    if (payload.type !== "conversation_snapshot") {
                        return
                    }
                    const snapshot = parseConversationSnapshotResponse(payload.state, "/api/conversations/{id}/events")
                    applyConversationSnapshotRef.current?.(activeProjectPath, snapshot)
                } catch {
                    // Ignore malformed stream events.
                }
            }
        }

        return () => {
            isCancelled = true
            eventSource?.close()
        }
    }, [activeConversationId, activeProjectPath, hasLoadedActiveConversationSnapshot])

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
        if (result.ok) {
            setProjectRegistrationError(null)
        }
    }

    const onOpenProjectDirectoryChooser = () => {
        clearProjectRegistrationError()
        if (!projectDirectoryPickerInputRef.current) {
            setProjectRegistrationError("Directory picker is unavailable. Enter an absolute path manually.")
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
        const selectedProjectPath = deriveProjectPathFromDirectorySelection(event.target.files)
        event.target.value = ""
        if (!selectedProjectPath) {
            setProjectRegistrationError(
                "Unable to resolve an absolute project path from the selected directory. Enter an absolute path manually.",
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
        activateConversationThread(activeProjectPath, conversationId)
    }

    const onSelectConversationThread = (conversationId: string) => {
        if (!activeProjectPath) {
            return
        }
        setPanelError(null)
        activateConversationThread(activeProjectPath, conversationId)
        const cachedSnapshot = projectConversationSnapshots[conversationId]
        if (cachedSnapshot) {
            applyConversationSnapshot(activeProjectPath, cachedSnapshot)
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
        activateConversationThread(activeProjectPath, conversationId)
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
        const priorConversationId = activeConversationId
        const priorConversationHistory = activeConversationHistory
        const optimisticUserTurn: ConversationHistoryEntry = {
            role: "user",
            content: trimmed,
            timestamp: new Date().toISOString(),
            kind: "message",
            artifactId: null,
        }

        setIsSendingChat(true)
        setPanelError(null)
        setChatDraft("")
        appendConversationHistoryEntry(optimisticUserTurn)
        try {
            const snapshot = await sendConversationTurnValidated(conversationId, {
                project_path: activeProjectPath,
                message: trimmed,
                model: model.trim() || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot)
        } catch (error) {
            updateProjectScopedWorkspace(activeProjectPath, {
                conversationId: priorConversationId,
                conversationHistory: priorConversationHistory,
            })
            const message = extractApiErrorMessage(error, "Unable to send the project chat turn.")
            setPanelError(message)
            appendLocalProjectEvent(`Project chat turn failed: ${message}`)
        } finally {
            setIsSendingChat(false)
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
            applyConversationSnapshot(activeProjectPath, snapshot)
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
            applyConversationSnapshot(activeProjectPath, snapshot)
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
            applyConversationSnapshot(activeProjectPath, snapshot)
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
                <div className="space-y-1">
                    <h2 className="text-lg font-semibold">Home</h2>
                    <p className="text-sm text-muted-foreground">
                        Project selection and AI collaboration start in this workspace.
                    </p>
                </div>
                <div className="rounded-md border border-border bg-card p-4 text-sm text-muted-foreground shadow-sm">
                    Home is now the first-class workspace for project-scoped conversation, spec edit review, and workflow planning.
                </div>
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
                                        <p className="text-xs text-muted-foreground">
                                            Select the active project context for chat and workflow activity.
                                        </p>
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
                                        <ul data-testid="projects-list" className="space-y-2">
                                            {orderedProjects.length === 0 ? (
                                                <li className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                                                    No projects registered yet.
                                                </li>
                                            ) : (
                                                orderedProjects.map((project) => {
                                                    const projectPath = project.directoryPath
                                                    const isActive = projectPath === activeProjectPath
                                                    return (
                                                        <li key={projectPath}>
                                                            <button
                                                                type="button"
                                                                onClick={() => {
                                                                    void onActivateProject(projectPath)
                                                                }}
                                                                aria-current={isActive ? "true" : undefined}
                                                                className={`w-full rounded border px-3 py-2 text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActive
                                                                    ? "border-primary/60 bg-primary/10 text-foreground"
                                                                    : "border-border hover:bg-muted"
                                                                    }`}
                                                            >
                                                                <div className="flex items-start justify-between gap-2">
                                                                    <div className="min-w-0 space-y-1">
                                                                        <p className="truncate text-xs font-medium text-foreground">
                                                                            {formatProjectListLabel(projectPath)}
                                                                        </p>
                                                                        <p className="truncate text-[11px] text-muted-foreground">
                                                                            {projectPath}
                                                                        </p>
                                                                    </div>
                                                                    {isActive ? (
                                                                        <span className="rounded bg-primary/20 px-2 py-0.5 text-[10px] font-semibold text-primary">
                                                                            Active
                                                                        </span>
                                                                    ) : null}
                                                                </div>
                                                            </button>
                                                        </li>
                                                    )
                                                })
                                            )}
                                        </ul>
                                        {activeProjectPath ? (
                                            <div className="mt-4 border-t border-border pt-4">
                                                <div
                                                    data-testid="project-thread-controls"
                                                    className="flex items-start justify-between gap-2"
                                                >
                                                    <div className="space-y-1">
                                                        <h4 className="text-sm font-semibold text-foreground">Threads</h4>
                                                        <p className="text-[11px] text-muted-foreground">
                                                            Separate conversation history within this project.
                                                        </p>
                                                    </div>
                                                    <button
                                                        data-testid="project-thread-new-button"
                                                        type="button"
                                                        onClick={onCreateConversationThread}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        New thread
                                                    </button>
                                                </div>
                                                <ul data-testid="project-thread-list" className="mt-3 space-y-2">
                                                    {activeProjectConversationSummaries.length === 0 ? (
                                                        <li className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                                                            No threads yet.
                                                        </li>
                                                    ) : (
                                                        activeProjectConversationSummaries.map((conversation) => {
                                                            const isActiveConversation = conversation.conversation_id === activeConversationId
                                                            return (
                                                                <li key={conversation.conversation_id}>
                                                                    <button
                                                                        type="button"
                                                                        onClick={() => onSelectConversationThread(conversation.conversation_id)}
                                                                        aria-current={isActiveConversation ? "true" : undefined}
                                                                        className={`w-full rounded border px-3 py-2 text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActiveConversation
                                                                            ? "border-primary/60 bg-primary/10 text-foreground"
                                                                            : "border-border hover:bg-muted"
                                                                            }`}
                                                                    >
                                                                        <div className="flex items-start justify-between gap-2">
                                                                            <div className="min-w-0 space-y-1">
                                                                                <p className="truncate text-xs font-medium text-foreground">
                                                                                    {conversation.title}
                                                                                </p>
                                                                                <p className="truncate text-[11px] text-muted-foreground">
                                                                                    {conversation.last_message_preview || formatConversationTimestamp(conversation.updated_at)}
                                                                                </p>
                                                                            </div>
                                                                            {isActiveConversation ? (
                                                                                <span className="rounded bg-primary/20 px-2 py-0.5 text-[10px] font-semibold text-primary">
                                                                                    Active
                                                                                </span>
                                                                            ) : null}
                                                                        </div>
                                                                    </button>
                                                                </li>
                                                            )
                                                        })
                                                    )}
                                                </ul>
                                            </div>
                                        ) : null}
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
                                                        const key = `${entry.timestamp}-${entry.artifactId || index}`
                                                        if (entry.kind === "tool_call" && entry.toolCall) {
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

                                                        if (entry.kind === "spec_edit_proposal" && entry.artifactId) {
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

                                                        if (entry.kind === "execution_card" && entry.artifactId) {
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

                                                        if (entry.role !== "user" && entry.role !== "assistant") {
                                                            return null
                                                        }

                                                        return (
                                                            <li
                                                                key={key}
                                                                className={`flex ${entry.role === "user" ? "justify-end" : "justify-start"}`}
                                                            >
                                                                <div
                                                                    className={`max-w-[85%] rounded border px-3 py-2 ${entry.role === "user"
                                                                        ? "border-primary/40 bg-primary/10 text-foreground"
                                                                        : "border-border bg-muted/40 text-foreground"
                                                                        }`}
                                                                >
                                                                    <p className="text-[10px] font-semibold uppercase tracking-wide opacity-70">
                                                                        {entry.role === "assistant" ? "Attractor" : entry.role}
                                                                    </p>
                                                                    <p className="whitespace-pre-wrap text-xs leading-5">{entry.content}</p>
                                                                    <p className="mt-1 text-[10px] opacity-70">{formatConversationTimestamp(entry.timestamp)}</p>
                                                                </div>
                                                            </li>
                                                        )
                                                    })}
                                                    {showPendingAssistantRow ? (
                                                        <li
                                                            data-testid="project-ai-conversation-pending-row"
                                                            className="flex justify-start"
                                                        >
                                                            <div className="max-w-[85%] rounded border border-border bg-muted/30 px-3 py-2 text-foreground">
                                                                <p className="text-[10px] font-semibold uppercase tracking-wide opacity-70">
                                                                    Attractor
                                                                </p>
                                                                <p className="text-xs leading-5 text-muted-foreground">
                                                                    Thinking...
                                                                </p>
                                                            </div>
                                                        </li>
                                                    ) : null}
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
                                                {isSendingChat ? "Thinking..." : "Send"}
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
