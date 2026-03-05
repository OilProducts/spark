import { type ConversationHistoryEntry, type PlanStatus, type ProjectRegistrationResult, useStore } from "@/store"
import { type ChangeEvent, type FormEvent, type KeyboardEvent, type PointerEvent as ReactPointerEvent, useEffect, useRef, useState } from "react"
import { ChevronDown, ChevronUp } from "lucide-react"
import { buildPipelineStartPayload } from "@/lib/pipelineStartPayload"
import { ApiHttpError, fetchFlowPayloadValidated, fetchPipelineStartValidated, fetchPipelineStatusValidated } from '@/lib/apiClient'
import { useNarrowViewport } from "@/lib/useNarrowViewport"
import { isAbsoluteProjectPath, normalizeProjectPath } from "@/lib/projectPaths"
import { HomeProjectSidebar } from "@/components/HomeProjectSidebar"
import { HomeWorkspace } from "@/components/HomeWorkspace"
import {
    clearProjectSpecEditProposal,
    getProjectSpecEditProposal,
    type ProjectSpecEditProposalMap,
    type SpecEditProposalPreview,
    upsertProjectSpecEditProposal,
} from "@/lib/projectSpecProposals"

const buildProjectScopedArtifactId = (artifactType: "conversation" | "spec" | "plan", projectPath: string) => {
    const normalizedProjectKey = projectPath
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/(^-|-$)/g, "")
    const suffix = normalizedProjectKey || "project"
    return `${artifactType}-${suffix}-${Date.now()}`
}

const PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT = 12
const DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT = 320
const HOME_SIDEBAR_MIN_PRIMARY_HEIGHT = 208
const HOME_SIDEBAR_MIN_SECONDARY_HEIGHT = 208
const HOME_SIDEBAR_RESIZE_HANDLE_HEIGHT = 12

const PLAN_STATUS_TRANSITIONS: Record<PlanStatus, PlanStatus[]> = {
    draft: ['approved', 'rejected', 'revision-requested'],
    approved: ['rejected', 'revision-requested'],
    rejected: ['revision-requested', 'approved'],
    'revision-requested': ['approved', 'rejected'],
}

const PLAN_TRANSITION_ACTION_LABELS: Record<PlanStatus, string> = {
    draft: 'Reset',
    approved: 'Approved',
    rejected: 'Rejected',
    'revision-requested': 'Requested revision for',
}

const canTransitionPlanStatus = (from: PlanStatus, to: PlanStatus) =>
    from !== to && PLAN_STATUS_TRANSITIONS[from].includes(to)

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

type WorkflowFailureDiagnostics = {
    message: string
    failedAt: string
    flowSource: string | null
}

type ProjectGitMetadata = {
    branch: string | null
    commit: string | null
}

const EMPTY_PROJECT_GIT_METADATA: ProjectGitMetadata = {
    branch: null,
    commit: null,
}

const asProjectGitMetadataField = (value: unknown): string | null => {
    if (typeof value !== "string") {
        return null
    }
    const trimmed = value.trim()
    return trimmed.length > 0 ? trimmed : null
}

type PickerFileWithPath = File & {
    path?: string
    webkitRelativePath?: string
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

const formatProjectListLabel = (projectPath: string) => {
    const normalizedPath = normalizeProjectPath(projectPath)
    const segments = normalizedPath.split('/').filter(Boolean)
    if (segments.length === 0) {
        return normalizedPath
    }
    return segments[segments.length - 1]
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
    const setSpecId = useStore((state) => state.setSpecId)
    const setSpecStatus = useStore((state) => state.setSpecStatus)
    const setSpecProvenance = useStore((state) => state.setSpecProvenance)
    const setPlanId = useStore((state) => state.setPlanId)
    const setPlanStatus = useStore((state) => state.setPlanStatus)
    const setPlanProvenance = useStore((state) => state.setPlanProvenance)
    const activeFlow = useStore((state) => state.activeFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const workingDir = useStore((state) => state.workingDir)
    const model = useStore((state) => state.model)
    const [projectGitMetadata, setProjectGitMetadata] = useState<Record<string, ProjectGitMetadata>>({})
    const [projectSpecEditProposals, setProjectSpecEditProposals] = useState<ProjectSpecEditProposalMap>({})
    const [planGenerationError, setPlanGenerationError] = useState<string | null>(null)
    const [planGenerationStatusDegraded, setPlanGenerationStatusDegraded] = useState<string | null>(null)
    const [lastPlanGenerationFailure, setLastPlanGenerationFailure] = useState<WorkflowFailureDiagnostics | null>(null)
    const [isPlanGenerationLaunching, setIsPlanGenerationLaunching] = useState(false)
    const projectDirectoryPickerInputRef = useRef<HTMLInputElement | null>(null)
    const homeSidebarRef = useRef<HTMLDivElement | null>(null)
    const homeSidebarResizeRef = useRef<{ startY: number; startHeight: number } | null>(null)
    const [chatDraft, setChatDraft] = useState("")
    const [expandedProposalChanges, setExpandedProposalChanges] = useState<Record<string, boolean>>({})
    const [homeSidebarPrimaryHeight, setHomeSidebarPrimaryHeight] = useState(DEFAULT_HOME_SIDEBAR_PRIMARY_HEIGHT)
    const [isHomeSidebarResizing, setIsHomeSidebarResizing] = useState(false)
    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null
    const activeProjectGitMetadata = activeProjectPath
        ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
        : EMPTY_PROJECT_GIT_METADATA
    const activeProjectProposalPreview = getProjectSpecEditProposal(projectSpecEditProposals, activeProjectPath)
    const specIsApprovedForPlanning = activeProjectScope?.specStatus === 'approved'
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
    const activeConversationHistory = activeProjectScope?.conversationHistory || []
    const activeChatHistory = activeConversationHistory.filter(
        (entry) => entry.role === "user" || entry.role === "assistant"
    )
    const activeProjectEventLog = activeProjectScope?.projectEventLog || []
    const activePlanStatus: PlanStatus = activeProjectScope?.planStatus || 'draft'
    const canRerunPlanGeneration = Boolean(activeProjectScope?.specId) && specIsApprovedForPlanning && Boolean(activeFlow)

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
                })
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
    }, [projects, projectGitMetadata])

    useEffect(() => {
        setChatDraft("")
    }, [activeProjectPath])

    useEffect(() => {
        setExpandedProposalChanges({})
    }, [activeProjectPath, activeProjectProposalPreview?.id])

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
        window.addEventListener('resize', syncSidebarHeight)
        return () => {
            window.removeEventListener('resize', syncSidebarHeight)
        }
    }, [isNarrowViewport])

    useEffect(() => {
        if (!isHomeSidebarResizing) {
            return
        }

        const stopHomeSidebarResize = () => {
            setIsHomeSidebarResizing(false)
            homeSidebarResizeRef.current = null
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
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

        window.addEventListener('pointermove', handleHomeSidebarPointerMove)
        window.addEventListener('pointerup', stopHomeSidebarResize)
        window.addEventListener('pointercancel', stopHomeSidebarResize)
        return () => {
            window.removeEventListener('pointermove', handleHomeSidebarPointerMove)
            window.removeEventListener('pointerup', stopHomeSidebarResize)
            window.removeEventListener('pointercancel', stopHomeSidebarResize)
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
        }
    }, [isHomeSidebarResizing])

    const resolveProjectPathValidation = (rawPath: string): ProjectRegistrationResult => {
        const normalizedPath = normalizeProjectPath(rawPath)
        if (!normalizedPath) {
            return { ok: false, error: 'Project directory path is required.' }
        }
        if (!isAbsoluteProjectPath(normalizedPath)) {
            return {
                ok: false,
                normalizedPath,
                error: 'Project directory path must be absolute.',
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
                let message = 'Unable to verify project Git state.'
                try {
                    const payload = (await response.json()) as { detail?: string }
                    if (payload?.detail) {
                        message = payload.detail
                    }
                } catch {
                    // ignore
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
            return { metadata: { ...EMPTY_PROJECT_GIT_METADATA }, error: 'Unable to verify project Git state.' }
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
            setProjectRegistrationError('Project directory must be a Git repository.')
            return null
        }
        return metadata
    }

    const registerProjectFromPath = async (rawProjectPath: string) => {
        const validation = resolveProjectPathValidation(rawProjectPath)
        if (!validation.ok || !validation.normalizedPath) {
            setProjectRegistrationError(validation.error ?? 'Project directory path is required.')
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
            setProjectRegistrationError('Directory picker is unavailable. Enter an absolute path manually.')
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
        document.body.style.cursor = 'row-resize'
        document.body.style.userSelect = 'none'
        event.preventDefault()
    }

    const onHomeSidebarResizeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === 'ArrowUp') {
            event.preventDefault()
            adjustHomeSidebarPrimaryHeight(-24)
            return
        }
        if (event.key === 'ArrowDown') {
            event.preventDefault()
            adjustHomeSidebarPrimaryHeight(24)
            return
        }
        if (event.key === 'Home') {
            event.preventDefault()
            const containerHeight = homeSidebarRef.current?.getBoundingClientRect().height || 0
            if (containerHeight <= 0) {
                return
            }
            setHomeSidebarPrimaryHeight(clampHomeSidebarPrimaryHeight(HOME_SIDEBAR_MIN_PRIMARY_HEIGHT, containerHeight))
            return
        }
        if (event.key === 'End') {
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
                'Unable to resolve an absolute project path from the selected directory. Enter an absolute path manually.',
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

    const ensureConversationId = () => {
        if (!activeProjectPath) {
            return null
        }
        if (activeProjectScope?.conversationId) {
            return activeProjectScope.conversationId
        }
        const conversationId = buildProjectScopedArtifactId("conversation", activeProjectPath)
        setConversationId(conversationId)
        return conversationId
    }

    const onSendChatMessage = () => {
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

        const userEntry: ConversationHistoryEntry = {
            role: "user",
            content: trimmed,
            timestamp: new Date().toISOString(),
        }
        appendConversationHistoryEntry(userEntry)

        const summarizedIntent = trimmed.length > 120 ? `${trimmed.slice(0, 117)}...` : trimmed
        const assistantEntry: ConversationHistoryEntry = {
            role: "assistant",
            content: `Acknowledged: "${summarizedIntent}". I drafted a spec edit proposal below for your review.`,
            timestamp: new Date().toISOString(),
        }
        appendConversationHistoryEntry(assistantEntry)
        upsertAgentSpecEditProposal(trimmed)
        setChatDraft("")
    }

    const onChatComposerSubmit = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        onSendChatMessage()
    }

    const onChatComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
        if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault()
            onSendChatMessage()
        }
    }

    const formatConversationTimestamp = (value: string) => {
        const parsed = new Date(value)
        if (Number.isNaN(parsed.getTime())) {
            return value
        }
        return parsed.toLocaleString()
    }

    const appendProjectEvent = (message: string) => {
        appendProjectEventEntry({
            message,
            timestamp: new Date().toISOString(),
        })
    }

    const buildProposalDiffLines = (change: SpecEditProposalPreview["changes"][number]) => {
        const beforeLines = change.before.split('\n').map((line) => ({ type: "removed" as const, text: line }))
        const afterLines = change.after.split('\n').map((line) => ({ type: "added" as const, text: line }))
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

    const truncateProposalSource = (value: string, maxLength = 72) => {
        if (value.length <= maxLength) {
            return value
        }
        return `${value.slice(0, maxLength - 1)}...`
    }

    const buildAgentSpecEditProposal = (sourceText: string): SpecEditProposalPreview => {
        const proposal: SpecEditProposalPreview = {
            id: `proposal-${Date.now()}`,
            createdAt: new Date().toISOString(),
            summary: "Agent-proposed spec refinements generated from the latest project-scoped conversation turn.",
            changes: [
                {
                    path: "spec/goals.md#scope",
                    before: "Document high-level feature scope.",
                    after: `Document scope anchored to: ${truncateProposalSource(sourceText)}`
                },
                {
                    path: "spec/acceptance.md#checks",
                    before: "List acceptance checks for UI behavior.",
                    after: "List acceptance checks for project-scoped proposal preview and explicit apply gating."
                },
            ],
        }
        return proposal
    }

    const upsertAgentSpecEditProposal = (sourceText: string) => {
        if (!activeProjectPath) {
            return
        }
        const proposal = buildAgentSpecEditProposal(sourceText)
        setProjectSpecEditProposals((current) => upsertProjectSpecEditProposal(current, activeProjectPath, proposal))
        appendProjectEvent(`Agent proposed spec edits ${proposal.id}.`)
    }

    const onApplySpecEditProposal = () => {
        if (!activeProjectPath || !activeProjectProposalPreview) {
            return
        }
        if (!window.confirm('Apply these proposed spec edits to the active project spec?')) {
            return
        }

        const specId = activeProjectScope?.specId || buildProjectScopedArtifactId("spec", activeProjectPath)
        setSpecId(specId)
        setSpecStatus('approved')
        setSpecProvenance({
            source: "spec-edit-proposal",
            referenceId: activeProjectProposalPreview.id,
            capturedAt: new Date().toISOString(),
            runId: activeProjectScope?.artifactRunId || null,
            gitBranch: activeProjectGitMetadata.branch,
            gitCommit: activeProjectGitMetadata.commit,
        })
        appendProjectEvent(`Applied spec edit proposal ${activeProjectProposalPreview.id} to ${specId}.`)
        setProjectSpecEditProposals((current) => clearProjectSpecEditProposal(current, activeProjectPath))
        void onLaunchPlanGenerationWorkflow({
            specIdOverride: specId,
            trigger: "spec-proposal-apply",
        })
    }

    const onLaunchPlanGenerationWorkflow = async (options?: {
        specIdOverride?: string | null
        trigger?: "spec-proposal-apply" | "retry"
    }) => {
        const effectiveSpecId = options?.specIdOverride || activeProjectScope?.specId
        if (!activeProjectPath || !effectiveSpecId) {
            return
        }
        if (!activeFlow) {
            setPlanGenerationError('Select a plan-generation flow before launching.')
            appendProjectEvent('Plan-generation launch blocked: no active flow selected.')
            return
        }

        setPlanGenerationError(null)
        setPlanGenerationStatusDegraded(null)
        setIsPlanGenerationLaunching(true)
        try {
            const flow = await fetchFlowPayloadValidated(activeFlow)

            const runInitiationForm = {
                projectPath: activeProjectPath,
                flowSource: activeFlow,
                workingDirectory: workingDir.trim() || activeProjectPath,
                backend: 'codex',
                model: model.trim() || null,
                specArtifactId: effectiveSpecId,
                planArtifactId: activeProjectScope?.planId || null,
            }
            const startPayload = buildPipelineStartPayload(runInitiationForm, flow.content)
            const runData = await fetchPipelineStartValidated(startPayload as Record<string, unknown>)
            if (typeof runData?.pipeline_id !== 'string') {
                throw new Error('Plan-generation launch did not return a pipeline id.')
            }

            try {
                await fetchPipelineStatusValidated(runData.pipeline_id)
            } catch (statusError) {
                const detail = statusError instanceof ApiHttpError && statusError.detail
                    ? statusError.detail
                    : statusError instanceof Error
                        ? statusError.message
                        : 'Plan status retrieval unavailable.'
                setPlanGenerationStatusDegraded(`Plan generation launched, but status retrieval is degraded: ${detail}`)
                appendProjectEvent(`Plan generation launched with degraded status retrieval: ${detail}`)
            }

            setSelectedRunId(runData.pipeline_id)
            setPlanId(activeProjectScope.planId || buildProjectScopedArtifactId("plan", activeProjectPath))
            setPlanStatus('draft')
            setPlanProvenance({
                source: "plan-generation-workflow",
                referenceId: runData.pipeline_id,
                capturedAt: new Date().toISOString(),
                runId: runData.pipeline_id,
                gitBranch: activeProjectGitMetadata.branch,
                gitCommit: activeProjectGitMetadata.commit,
            })
            const triggerSource = options?.trigger === "retry" ? "retry" : "approved spec edit"
            appendProjectEvent(`Launched plan-generation workflow from ${triggerSource} ${effectiveSpecId}.`)
            setLastPlanGenerationFailure(null)
        } catch (error) {
            const message = error instanceof ApiHttpError && error.detail
                ? error.detail
                : error instanceof Error
                    ? error.message
                    : 'Failed to launch plan-generation workflow.'
            setPlanGenerationError(message)
            setPlanGenerationStatusDegraded(null)
            setLastPlanGenerationFailure({
                message,
                failedAt: new Date().toISOString(),
                flowSource: activeFlow || null,
            })
            appendProjectEvent(`Plan-generation workflow launch failed: ${message}`)
        } finally {
            setIsPlanGenerationLaunching(false)
        }
    }

    const onPlanGateTransition = (nextStatus: PlanStatus) => {
        if (!activeProjectPath || !activeProjectScope?.planId) {
            setPlanGenerationError('Create or open a plan before using plan gate controls.')
            return
        }
        if (!canTransitionPlanStatus(activeProjectScope.planStatus, nextStatus)) {
            setPlanGenerationError(
                `Cannot transition plan status from ${activeProjectScope.planStatus} to ${nextStatus}.`
            )
            return
        }
        setPlanGenerationError(null)
        setPlanGenerationStatusDegraded(null)
        const previousStatus = activeProjectScope.planStatus
        const transitionAction = PLAN_TRANSITION_ACTION_LABELS[nextStatus]
        setPlanStatus(nextStatus)
        appendProjectEvent(`${transitionAction} plan ${activeProjectScope.planId} (${previousStatus} -> ${nextStatus}).`)
    }

    const onRejectSpecEditProposal = () => {
        if (!activeProjectPath || !activeProjectProposalPreview) {
            return
        }

        appendProjectEvent(`Rejected spec edit proposal ${activeProjectProposalPreview.id}.`)
        setProjectSpecEditProposals((current) => clearProjectSpecEditProposal(current, activeProjectPath))
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
                    Home is now the first-class workspace for project-scoped conversation, spec proposals, and orchestration launch.
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
                            <div className="mb-3 space-y-1">
                                <h3 className="text-sm font-semibold text-foreground">Project-Scoped AI Conversation</h3>
                                <p className="text-xs text-muted-foreground">
                                    Chat with the project AI directly in this thread. Messages stay scoped to the active project.
                                </p>
                            </div>
                            {!activeProjectPath ? (
                                <p className={`rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground ${isNarrowViewport ? "" : "flex flex-1 items-center"}`}>
                                    Select an active project to begin chatting.
                                </p>
                            ) : (
                                <div className="flex min-h-0 flex-1 flex-col gap-3">
                                    <div
                                        data-testid="project-ai-conversation-body"
                                        className={`flex min-h-0 flex-1 flex-col gap-3 ${isNarrowViewport ? "" : "overflow-y-auto pr-1"}`}
                                    >
                                        <p className="truncate rounded-md border border-border px-3 py-2 text-xs text-muted-foreground">
                                            Active conversation artifact: {activeProjectScope?.conversationId || "Not created yet. Sending your first message creates one."}
                                        </p>
                                        <p className="text-[11px] text-muted-foreground">
                                            Conversation turns: <span className="font-medium text-foreground">{activeChatHistory.length}</span>
                                        </p>
                                        <div className="flex flex-wrap items-center gap-3">
                                            <p className="truncate text-xs text-muted-foreground">
                                                Spec artifact: <span className="font-mono text-foreground">{activeProjectScope?.specId || "Not created yet"}</span>
                                            </p>
                                            <p className="text-xs text-muted-foreground">
                                                Spec status: <span className="font-medium text-foreground">{activeProjectScope?.specStatus || "draft"}</span>
                                            </p>
                                        </div>
                                        <p className="text-[11px] text-muted-foreground">
                                            Spec edit proposals are emitted by the assistant and appear inline below the chat thread.
                                        </p>
                                        <div data-testid="project-ai-conversation-history" className="flex min-h-0 flex-col rounded-md border border-border px-3 py-2">
                                            <p className="text-xs font-medium text-foreground">Conversation history</p>
                                            <p className="mb-2 text-xs text-muted-foreground">
                                                Conversation history is scoped to the active project and remains discoverable when you return.
                                            </p>
                                            {activeChatHistory.length === 0 ? (
                                                <p className="text-xs text-muted-foreground">No conversation history for this project yet.</p>
                                            ) : (
                                                <ol data-testid="project-ai-conversation-history-list" className="space-y-2">
                                                    {activeChatHistory.map((entry, index) => (
                                                        <li
                                                            key={`${entry.timestamp}-${index}`}
                                                            className={`flex ${entry.role === "user" ? "justify-end" : "justify-start"}`}
                                                        >
                                                            <div
                                                                className={`max-w-[85%] rounded border px-3 py-2 ${entry.role === "user"
                                                                    ? "border-primary/40 bg-primary/10 text-foreground"
                                                                    : entry.role === "assistant"
                                                                        ? "border-border bg-muted/40 text-foreground"
                                                                        : "border-border bg-background text-muted-foreground"
                                                                    }`}
                                                            >
                                                                <p className="text-[10px] font-semibold uppercase tracking-wide opacity-70">
                                                                    {entry.role === "assistant" ? "AI" : entry.role}
                                                                </p>
                                                                <p className="whitespace-pre-wrap text-xs leading-5">{entry.content}</p>
                                                                <p className="mt-1 text-[10px] opacity-70">{formatConversationTimestamp(entry.timestamp)}</p>
                                                            </div>
                                                        </li>
                                                    ))}
                                                </ol>
                                            )}
                                        </div>
                                        {activeProjectProposalPreview ? (
                                            <div data-testid="project-spec-edit-proposal-preview" className="rounded-md border border-border px-3 py-2">
                                                <div className="flex flex-wrap items-center justify-between gap-2">
                                                    <p className="text-xs font-medium text-foreground">Spec edit proposal</p>
                                                    <span className="rounded bg-amber-500/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-800">
                                                        Pending review
                                                    </span>
                                                </div>
                                                <p className="text-[11px] text-muted-foreground">
                                                    Generated {formatConversationTimestamp(activeProjectProposalPreview.createdAt)} ({activeProjectProposalPreview.id})
                                                </p>
                                                <p className="mt-1 text-xs text-foreground">{activeProjectProposalPreview.summary}</p>
                                                <ul className="mt-2 space-y-2">
                                                    {activeProjectProposalPreview.changes.map((change, index) => {
                                                        const diffLines = buildProposalDiffLines(change)
                                                        const shouldCollapse = diffLines.length > PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT
                                                        const changeKey = buildProposalChangeKey(activeProjectProposalPreview.id, change.path, index)
                                                        const isExpanded = expandedProposalChanges[changeKey] === true
                                                        const visibleLines = shouldCollapse && !isExpanded
                                                            ? diffLines.slice(0, PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT)
                                                            : diffLines
                                                        return (
                                                            <li key={`${activeProjectProposalPreview.id}-${change.path}-${index}`} className="rounded border border-border">
                                                                <div className="flex items-center justify-between gap-2 border-b border-border px-2 py-1">
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
                                                                <div className="space-y-1 px-2 py-2">
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
                                                <div className="mt-3 flex flex-wrap items-center gap-2">
                                                    <button
                                                        data-testid="project-spec-edit-proposal-apply-button"
                                                        type="button"
                                                        onClick={onApplySpecEditProposal}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        Apply proposal
                                                    </button>
                                                    <button
                                                        data-testid="project-spec-edit-proposal-reject-button"
                                                        type="button"
                                                        onClick={onRejectSpecEditProposal}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        Reject proposal
                                                    </button>
                                                    <p className="text-[11px] text-muted-foreground">
                                                        Applying requires explicit confirmation.
                                                    </p>
                                                </div>
                                            </div>
                                        ) : null}
                                        {(activeProjectScope?.specId
                                            || activeProjectScope?.planId
                                            || planGenerationError
                                            || planGenerationStatusDegraded
                                            || lastPlanGenerationFailure
                                            || isPlanGenerationLaunching) ? (
                                            <div data-testid="project-plan-generation-surface" className="rounded-md border border-border px-3 py-2">
                                                <div className="mb-2 space-y-1">
                                                    <p className="text-xs font-medium text-foreground">Spec to Plan Workflow</p>
                                                    <p className="text-[11px] text-muted-foreground">
                                                        Launch is triggered automatically when a spec edit proposal is applied.
                                                    </p>
                                                </div>
                                                <p className="truncate text-xs text-muted-foreground">
                                                    Plan artifact: <span className="font-mono text-foreground">{activeProjectScope?.planId || "Not created yet"}</span>
                                                </p>
                                                <p className="text-xs text-muted-foreground">
                                                    Spec status: <span className="font-medium text-foreground">{specIsApprovedForPlanning ? "approved" : "draft"}</span>
                                                </p>
                                                <p className="text-xs text-muted-foreground">
                                                    Active flow source: <span className="font-mono text-foreground">{activeFlow || "none selected"}</span>
                                                </p>
                                                {isPlanGenerationLaunching ? (
                                                    <p data-testid="project-plan-generation-launching" className="text-[11px] text-muted-foreground">
                                                        Launching plan-generation workflow...
                                                    </p>
                                                ) : null}
                                                {!activeProjectScope?.planId && !isPlanGenerationLaunching ? (
                                                    <p className="text-[11px] text-muted-foreground">
                                                        Plan artifact will appear here once auto-launch finishes.
                                                    </p>
                                                ) : null}
                                                <div data-testid="project-plan-gate-surface" className="mt-2 rounded-md border border-border px-3 py-2">
                                                    <p className="text-xs font-medium text-foreground">Plan gate controls</p>
                                                    <p className="text-[11px] text-muted-foreground">
                                                        Plan status: <span className="font-medium text-foreground">{activePlanStatus}</span>
                                                    </p>
                                                    {!activeProjectScope?.planId ? (
                                                        <p className="text-[11px] text-muted-foreground">
                                                            Waiting for plan-generation output before gate actions are available.
                                                        </p>
                                                    ) : null}
                                                    <div className="mt-2 flex flex-wrap items-center gap-2">
                                                        <button
                                                            data-testid="project-plan-approve-button"
                                                            type="button"
                                                            onClick={() => onPlanGateTransition('approved')}
                                                            disabled={!activeProjectScope?.planId || !canTransitionPlanStatus(activePlanStatus, 'approved')}
                                                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                        >
                                                            Approve plan
                                                        </button>
                                                        <button
                                                            data-testid="project-plan-reject-button"
                                                            type="button"
                                                            onClick={() => onPlanGateTransition('rejected')}
                                                            disabled={!activeProjectScope?.planId || !canTransitionPlanStatus(activePlanStatus, 'rejected')}
                                                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                        >
                                                            Reject plan
                                                        </button>
                                                        <button
                                                            data-testid="project-plan-request-revision-button"
                                                            type="button"
                                                            onClick={() => onPlanGateTransition('revision-requested')}
                                                            disabled={!activeProjectScope?.planId || !canTransitionPlanStatus(activePlanStatus, 'revision-requested')}
                                                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                        >
                                                            Request revision
                                                        </button>
                                                    </div>
                                                </div>
                                                {planGenerationError ? (
                                                    <p data-testid="project-plan-generation-error" className="mt-2 text-[11px] text-destructive">
                                                        {planGenerationError}
                                                    </p>
                                                ) : null}
                                                {planGenerationStatusDegraded ? (
                                                    <p data-testid="project-plan-generation-status-degraded" className="mt-2 text-[11px] text-amber-800">
                                                        {planGenerationStatusDegraded}
                                                    </p>
                                                ) : null}
                                                {lastPlanGenerationFailure ? (
                                                    <div data-testid="project-plan-failure-diagnostics" className="mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[11px] text-destructive">
                                                        <p className="font-medium">Last planning launch failure</p>
                                                        <p data-testid="project-plan-failure-message">{lastPlanGenerationFailure.message}</p>
                                                        <p>
                                                            Flow source: <span className="font-mono">{lastPlanGenerationFailure.flowSource || "none selected"}</span>
                                                        </p>
                                                        <p>
                                                            Failed at: {formatConversationTimestamp(lastPlanGenerationFailure.failedAt)}
                                                        </p>
                                                        <div className="mt-2 flex flex-wrap items-center gap-2">
                                                            <button
                                                                data-testid="project-plan-generation-rerun-button"
                                                                type="button"
                                                                onClick={() => {
                                                                    void onLaunchPlanGenerationWorkflow({ trigger: "retry" })
                                                                }}
                                                                disabled={!canRerunPlanGeneration}
                                                                className="rounded border border-destructive/40 bg-background px-2 py-1 text-[11px] font-medium text-destructive hover:bg-destructive/5 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                                            >
                                                                Retry plan-generation workflow
                                                            </button>
                                                            {!canRerunPlanGeneration ? (
                                                                <p data-testid="project-plan-generation-rerun-disabled-reason" className="text-[11px] text-destructive/90">
                                                                    Fix launch prerequisites to enable rerun.
                                                                </p>
                                                            ) : null}
                                                        </div>
                                                    </div>
                                                ) : null}
                                            </div>
                                        ) : null}
                                        <p className="text-[11px] text-muted-foreground">
                                            Assistant messages and proposal cards are currently local UI placeholders until backend agent/tool APIs are connected.
                                        </p>
                                    </div>
                                    <form
                                        data-testid="project-ai-conversation-composer"
                                        onSubmit={onChatComposerSubmit}
                                        className="shrink-0 space-y-2 rounded-md border border-border px-3 py-3"
                                    >
                                        <label htmlFor="project-ai-conversation-input" className="text-xs font-medium text-foreground">
                                            Message
                                        </label>
                                        <textarea
                                            id="project-ai-conversation-input"
                                            data-testid="project-ai-conversation-input"
                                            value={chatDraft}
                                            onChange={(event) => setChatDraft(event.target.value)}
                                            onKeyDown={onChatComposerKeyDown}
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
                                                disabled={chatDraft.trim().length === 0}
                                                className="rounded border border-border px-3 py-1.5 text-xs font-medium hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                            >
                                                Send
                                            </button>
                                        </div>
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
