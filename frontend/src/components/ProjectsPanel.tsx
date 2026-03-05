import { type ConversationHistoryEntry, type PlanStatus, useStore } from "@/store"
import { type FormEvent, useEffect, useState } from "react"
import { buildPipelineStartPayload } from "@/lib/pipelineStartPayload"
import { ApiHttpError, fetchFlowPayloadValidated, fetchPipelineStartValidated, fetchPipelineStatusValidated } from '@/lib/apiClient'
import { useNarrowViewport } from "@/lib/useNarrowViewport"
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

export function ProjectsPanel() {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const projects = Object.values(projectRegistry)
    const recentProjectPaths = useStore((state) => state.recentProjectPaths)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectScopedWorkspaces = useStore((state) => state.projectScopedWorkspaces)
    const projectRegistrationError = useStore((state) => state.projectRegistrationError)
    const registerProject = useStore((state) => state.registerProject)
    const updateProjectPath = useStore((state) => state.updateProjectPath)
    const toggleProjectFavorite = useStore((state) => state.toggleProjectFavorite)
    const clearProjectRegistrationError = useStore((state) => state.clearProjectRegistrationError)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setConversationId = useStore((state) => state.setConversationId)
    const appendConversationHistoryEntry = useStore((state) => state.appendConversationHistoryEntry)
    const setSpecId = useStore((state) => state.setSpecId)
    const setSpecStatus = useStore((state) => state.setSpecStatus)
    const setSpecProvenance = useStore((state) => state.setSpecProvenance)
    const setPlanId = useStore((state) => state.setPlanId)
    const setPlanStatus = useStore((state) => state.setPlanStatus)
    const setPlanProvenance = useStore((state) => state.setPlanProvenance)
    const activeFlow = useStore((state) => state.activeFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)
    const workingDir = useStore((state) => state.workingDir)
    const model = useStore((state) => state.model)
    const [directoryPathInput, setDirectoryPathInput] = useState("")
    const [editingProjectPath, setEditingProjectPath] = useState<string | null>(null)
    const [editingDirectoryPathInput, setEditingDirectoryPathInput] = useState("")
    const [projectGitMetadata, setProjectGitMetadata] = useState<Record<string, ProjectGitMetadata>>({})
    const [projectSpecEditProposals, setProjectSpecEditProposals] = useState<ProjectSpecEditProposalMap>({})
    const [planGenerationError, setPlanGenerationError] = useState<string | null>(null)
    const [planGenerationStatusDegraded, setPlanGenerationStatusDegraded] = useState<string | null>(null)
    const [lastPlanGenerationFailure, setLastPlanGenerationFailure] = useState<WorkflowFailureDiagnostics | null>(null)
    const isNarrowViewport = useNarrowViewport()
    const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null
    const activeProjectGitMetadata = activeProjectPath
        ? projectGitMetadata[activeProjectPath] || EMPTY_PROJECT_GIT_METADATA
        : EMPTY_PROJECT_GIT_METADATA
    const activeProjectProposalPreview = getProjectSpecEditProposal(projectSpecEditProposals, activeProjectPath)
    const specIsApprovedForPlanning = activeProjectScope?.specStatus === 'approved'
    const favoriteProjects = projects.filter((project) => project.isFavorite)
    const recentProjects = recentProjectPaths
        .map((projectPath) => projectRegistry[projectPath])
        .filter((project): project is (typeof projects)[number] => Boolean(project))
    const activeConversationHistory = activeProjectScope?.conversationHistory || []
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

    const formatLastActivity = (value: string | null) => {
        if (!value) {
            return "No activity yet"
        }
        const parsed = new Date(value)
        if (Number.isNaN(parsed.getTime())) {
            return "Unknown activity time"
        }
        return parsed.toLocaleString()
    }

    const onRegisterProject = () => {
        const result = registerProject(directoryPathInput)
        if (result.ok) {
            setDirectoryPathInput("")
        }
    }

    const onSubmitProjectRegistration = (event: FormEvent<HTMLFormElement>) => {
        event.preventDefault()
        onRegisterProject()
    }

    const onStartProjectPathEdit = (projectPath: string) => {
        setEditingProjectPath(projectPath)
        setEditingDirectoryPathInput(projectPath)
        clearProjectRegistrationError()
    }

    const onCancelProjectPathEdit = () => {
        setEditingProjectPath(null)
        setEditingDirectoryPathInput("")
        clearProjectRegistrationError()
    }

    const onOpenConversation = () => {
        if (!activeProjectPath) {
            return
        }
        setConversationId(activeProjectScope?.conversationId || buildProjectScopedArtifactId("conversation", activeProjectPath))
    }

    const onStartConversation = () => {
        if (!activeProjectPath) {
            return
        }
        const conversationId = buildProjectScopedArtifactId("conversation", activeProjectPath)
        setConversationId(conversationId)
        const entry: ConversationHistoryEntry = {
            role: "system",
            content: `Started conversation ${conversationId}.`,
            timestamp: new Date().toISOString(),
        }
        appendConversationHistoryEntry(entry)
    }

    const onContinueConversation = () => {
        if (!activeProjectScope?.conversationId) {
            return
        }
        setConversationId(activeProjectScope.conversationId)
        const entry: ConversationHistoryEntry = {
            role: "system",
            content: `Continued conversation ${activeProjectScope.conversationId}.`,
            timestamp: new Date().toISOString(),
        }
        appendConversationHistoryEntry(entry)
    }

    const onOpenSpec = () => {
        if (!activeProjectPath) {
            return
        }
        setSpecId(activeProjectScope?.specId || buildProjectScopedArtifactId("spec", activeProjectPath))
        setSpecStatus(activeProjectScope?.specId ? activeProjectScope.specStatus : 'draft')
    }

    const onOpenPlan = () => {
        if (!activeProjectPath) {
            return
        }
        setPlanId(activeProjectScope?.planId || buildProjectScopedArtifactId("plan", activeProjectPath))
    }

    const formatConversationTimestamp = (value: string) => {
        const parsed = new Date(value)
        if (Number.isNaN(parsed.getTime())) {
            return value
        }
        return parsed.toLocaleString()
    }

    const truncateProposalSource = (value: string, maxLength = 72) => {
        if (value.length <= maxLength) {
            return value
        }
        return `${value.slice(0, maxLength - 1)}...`
    }

    const onPreviewSpecEditProposal = () => {
        if (!activeProjectPath) {
            return
        }
        const sourceEntry = [...activeConversationHistory].reverse().find((entry) => entry.role !== "system")
        const sourceText = sourceEntry?.content || "Capture baseline project scope and constraints."
        const proposal: SpecEditProposalPreview = {
            id: `proposal-${Date.now()}`,
            createdAt: new Date().toISOString(),
            summary: "Suggested spec refinements generated from the latest project-scoped conversation turn.",
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
        setProjectSpecEditProposals((current) => upsertProjectSpecEditProposal(current, activeProjectPath, proposal))
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
        setSpecStatus('draft')
        setSpecProvenance({
            source: "spec-edit-proposal",
            referenceId: activeProjectProposalPreview.id,
            capturedAt: new Date().toISOString(),
            runId: activeProjectScope?.artifactRunId || null,
            gitBranch: activeProjectGitMetadata.branch,
            gitCommit: activeProjectGitMetadata.commit,
        })
        appendConversationHistoryEntry({
            role: "system",
            content: `Applied spec edit proposal ${activeProjectProposalPreview.id} to ${specId}.`,
            timestamp: new Date().toISOString(),
        })
        setProjectSpecEditProposals((current) => clearProjectSpecEditProposal(current, activeProjectPath))
    }

    const onApproveSpecForPlanning = () => {
        if (!activeProjectPath || !activeProjectScope?.specId) {
            return
        }
        setSpecStatus('approved')
        setPlanGenerationError(null)
        setPlanGenerationStatusDegraded(null)
        appendConversationHistoryEntry({
            role: "system",
            content: `Approved spec ${activeProjectScope.specId} for plan generation.`,
            timestamp: new Date().toISOString(),
        })
    }

    const onLaunchPlanGenerationWorkflow = async () => {
        if (!activeProjectPath || !activeProjectScope?.specId || !specIsApprovedForPlanning) {
            return
        }
        if (!activeFlow) {
            setPlanGenerationError('Select a plan-generation flow before launching.')
            return
        }

        setPlanGenerationError(null)
        setPlanGenerationStatusDegraded(null)
        try {
            const flow = await fetchFlowPayloadValidated(activeFlow)

            const runInitiationForm = {
                projectPath: activeProjectPath,
                flowSource: activeFlow,
                workingDirectory: workingDir.trim() || activeProjectPath,
                backend: 'codex',
                model: model.trim() || null,
                specArtifactId: activeProjectScope?.specId || null,
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
            appendConversationHistoryEntry({
                role: "system",
                content: `Launched plan-generation workflow from approved spec ${activeProjectScope.specId}.`,
                timestamp: new Date().toISOString(),
            })
            setLastPlanGenerationFailure(null)
            setViewMode('execution')
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
        appendConversationHistoryEntry({
            role: "system",
            content: `${transitionAction} plan ${activeProjectScope.planId} (${previousStatus} -> ${nextStatus}).`,
            timestamp: new Date().toISOString(),
        })
    }

    const onRejectSpecEditProposal = () => {
        if (!activeProjectPath || !activeProjectProposalPreview) {
            return
        }

        appendConversationHistoryEntry({
            role: "system",
            content: `Rejected spec edit proposal ${activeProjectProposalPreview.id}.`,
            timestamp: new Date().toISOString(),
        })
        setProjectSpecEditProposals((current) => clearProjectSpecEditProposal(current, activeProjectPath))
    }

    return (
        <section
            data-testid="projects-panel"
            data-responsive-layout={isNarrowViewport ? "stacked" : "split"}
            className={`flex-1 overflow-auto ${isNarrowViewport ? "p-3" : "p-6"}`}
        >
            <div className="mx-auto w-full max-w-3xl space-y-6">
                <div className="space-y-1">
                    <h2 className="text-lg font-semibold">Projects</h2>
                    <p className="text-sm text-muted-foreground">
                        Project registration, selection, and workflow scoping live in this workspace.
                    </p>
                </div>
                <div className="rounded-md border border-border bg-card p-4 text-sm text-muted-foreground shadow-sm">
                    Projects workspace is now a first-class navigation area. Project registry and Git gating controls are tracked in the next checklist slices.
                </div>
                <div className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="mb-3 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Quick Switch</h3>
                        <p className="text-xs text-muted-foreground">Use favorites and recents to switch project context quickly.</p>
                    </div>
                    <div className="grid gap-4 md:grid-cols-2">
                        <div className="space-y-2">
                            <p className="text-xs font-medium text-foreground">Favorites</p>
                            <ul data-testid="favorite-projects-list" className="space-y-2">
                                {favoriteProjects.length === 0 ? (
                                    <li className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                                        No favorited projects yet.
                                    </li>
                                ) : (
                                    favoriteProjects.map((project) => {
                                        const projectPath = project.directoryPath
                                        const isActive = projectPath === activeProjectPath
                                        return (
                                            <li key={`favorite-${projectPath}`}>
                                                <button
                                                    type="button"
                                                    onClick={() => setActiveProjectPath(projectPath)}
                                                    aria-current={isActive ? "true" : undefined}
                                                    className={`w-full rounded border px-3 py-2 text-left text-xs focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActive
                                                        ? "border-primary/60 bg-primary/10 text-foreground"
                                                        : "border-border hover:bg-muted"
                                                        }`}
                                                >
                                                    <div className="flex items-center justify-between gap-2">
                                                        <span className="truncate">{projectPath}</span>
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
                        <div className="space-y-2">
                            <p className="text-xs font-medium text-foreground">Recent</p>
                            <ul data-testid="recent-projects-list" className="space-y-2">
                                {recentProjects.length === 0 ? (
                                    <li className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                                        No recent project switches yet.
                                    </li>
                                ) : (
                                    recentProjects.map((project) => {
                                        const projectPath = project.directoryPath
                                        const isActive = projectPath === activeProjectPath
                                        return (
                                            <li key={`recent-${projectPath}`}>
                                                <button
                                                    type="button"
                                                    onClick={() => setActiveProjectPath(projectPath)}
                                                    aria-current={isActive ? "true" : undefined}
                                                    className={`w-full rounded border px-3 py-2 text-left text-xs focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActive
                                                        ? "border-primary/60 bg-primary/10 text-foreground"
                                                        : "border-border hover:bg-muted"
                                                        }`}
                                                >
                                                    <div className="flex items-center justify-between gap-2">
                                                        <span className="truncate">{projectPath}</span>
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
                <div data-testid="project-scope-entrypoints" className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="mb-3 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Project-Scoped Entry Points</h3>
                        <p className="text-xs text-muted-foreground">
                            Conversation, spec, and plan artifacts are scoped to the active project.
                        </p>
                    </div>
                    {!activeProjectPath ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
                            Select an active project to access conversation, spec, and plan entry points.
                        </p>
                    ) : (
                        <div className="space-y-3">
                            <div data-testid="project-conversation-entrypoint" className="rounded-md border border-border px-3 py-2">
                                <div className="mb-2 flex items-center justify-between gap-3">
                                    <p className="text-sm font-medium text-foreground">Conversation</p>
                                    <button
                                        type="button"
                                        onClick={onOpenConversation}
                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    >
                                        {activeProjectScope?.conversationId ? "Open conversation" : "Start conversation"}
                                    </button>
                                </div>
                                <p className="truncate text-xs text-muted-foreground">
                                    {activeProjectScope?.conversationId || "No conversation artifact selected yet."}
                                </p>
                            </div>
                            <div data-testid="project-spec-entrypoint" className="rounded-md border border-border px-3 py-2">
                                <div className="mb-2 flex items-center justify-between gap-3">
                                    <p className="text-sm font-medium text-foreground">Spec</p>
                                    <button
                                        type="button"
                                        onClick={onOpenSpec}
                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    >
                                        {activeProjectScope?.specId ? "Open spec" : "Create spec"}
                                    </button>
                                </div>
                                <p className="truncate text-xs text-muted-foreground">
                                    {activeProjectScope?.specId || "No spec artifact selected yet."}
                                </p>
                            </div>
                            <div data-testid="project-plan-entrypoint" className="rounded-md border border-border px-3 py-2">
                                <div className="mb-2 flex items-center justify-between gap-3">
                                    <p className="text-sm font-medium text-foreground">Plan</p>
                                    <button
                                        type="button"
                                        onClick={onOpenPlan}
                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    >
                                        {activeProjectScope?.planId ? "Open plan" : "Create plan"}
                                    </button>
                                </div>
                                <p className="truncate text-xs text-muted-foreground">
                                    {activeProjectScope?.planId || "No plan artifact selected yet."}
                                </p>
                            </div>
                        </div>
                    )}
                </div>
                <div data-testid="project-ai-conversation-surface" className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="mb-3 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Project-Scoped AI Conversation</h3>
                        <p className="text-xs text-muted-foreground">
                            Start a new project conversation or continue an existing one in the active project scope.
                        </p>
                    </div>
                    {!activeProjectPath ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
                            Select an active project to start or continue a project-scoped AI conversation.
                        </p>
                    ) : (
                        <div className="space-y-3">
                            <p className="truncate rounded-md border border-border px-3 py-2 text-xs text-muted-foreground">
                                Active conversation artifact: {activeProjectScope?.conversationId || "No project conversation selected yet."}
                            </p>
                            <div className="flex flex-wrap items-center gap-2">
                                <button
                                    data-testid="project-ai-conversation-start-button"
                                    type="button"
                                    onClick={onStartConversation}
                                    className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                >
                                    Start conversation
                                </button>
                                <button
                                    data-testid="project-ai-conversation-continue-button"
                                    type="button"
                                    onClick={onContinueConversation}
                                    disabled={!activeProjectScope?.conversationId}
                                    className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                >
                                    Continue conversation
                                </button>
                            </div>
                            <div data-testid="project-ai-conversation-history" className="rounded-md border border-border px-3 py-2">
                                <p className="text-xs font-medium text-foreground">Conversation history</p>
                                <p className="mb-2 text-xs text-muted-foreground">
                                    Conversation history is scoped to the active project and remains discoverable when you return.
                                </p>
                                {activeConversationHistory.length === 0 ? (
                                    <p className="text-xs text-muted-foreground">No conversation history for this project yet.</p>
                                ) : (
                                    <ol data-testid="project-ai-conversation-history-list" className="space-y-2">
                                        {activeConversationHistory.map((entry, index) => (
                                            <li key={`${entry.timestamp}-${index}`} className="rounded border border-border px-2 py-1">
                                                <p className="text-[11px] text-muted-foreground">{formatConversationTimestamp(entry.timestamp)}</p>
                                                <p className="text-xs text-foreground">
                                                    <span className="font-medium">{entry.role}:</span> {entry.content}
                                                </p>
                                            </li>
                                        ))}
                                    </ol>
                                )}
                            </div>
                        </div>
                    )}
                </div>
                <div data-testid="project-spec-edit-proposal-surface" className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="mb-3 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Spec Edit Proposals</h3>
                        <p className="text-xs text-muted-foreground">
                            AI-generated spec edits appear here as explicit, reviewable proposals before any apply action.
                        </p>
                    </div>
                    {!activeProjectPath ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
                            Select an active project to review AI-generated spec edit proposals.
                        </p>
                    ) : (
                        <div className="space-y-3">
                            <div className="flex flex-wrap items-center gap-2">
                                <button
                                    data-testid="project-spec-edit-proposal-preview-button"
                                    type="button"
                                    onClick={onPreviewSpecEditProposal}
                                    className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                >
                                    Preview proposed spec edits
                                </button>
                            </div>
                            <p className="text-xs text-muted-foreground">
                                Proposal artifacts are scoped to the active project context.
                            </p>
                            {activeProjectProposalPreview ? (
                                <div data-testid="project-spec-edit-proposal-preview" className="rounded-md border border-border px-3 py-2">
                                    <p className="text-xs font-medium text-foreground">Proposal preview</p>
                                    <p className="text-[11px] text-muted-foreground">
                                        Generated {formatConversationTimestamp(activeProjectProposalPreview.createdAt)} ({activeProjectProposalPreview.id})
                                    </p>
                                    <p className="mt-1 text-xs text-foreground">{activeProjectProposalPreview.summary}</p>
                                    <ul className="mt-2 space-y-2">
                                        {activeProjectProposalPreview.changes.map((change) => (
                                            <li key={`${activeProjectProposalPreview.id}-${change.path}`} className="rounded border border-border px-2 py-1">
                                                <p className="text-[11px] font-medium text-foreground">{change.path}</p>
                                                <p className="text-[11px] text-muted-foreground">Before: {change.before}</p>
                                                <p className="text-[11px] text-foreground">After: {change.after}</p>
                                            </li>
                                        ))}
                                    </ul>
                                    <div className="mt-3 flex items-center gap-2">
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
                                            Applying proposed edits requires explicit confirmation. Rejecting a proposal dismisses it without mutating spec files.
                                        </p>
                                    </div>
                                </div>
                            ) : (
                                <p className="text-xs text-muted-foreground">No proposed spec edits for this project yet.</p>
                            )}
                        </div>
                    )}
                </div>
                <div data-testid="project-plan-generation-surface" className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="mb-3 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Spec to Plan Workflow Launch</h3>
                        <p className="text-xs text-muted-foreground">
                            Launch plan-generation workflows only after the active project spec is explicitly approved.
                        </p>
                    </div>
                    {!activeProjectPath ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
                            Select an active project to approve a spec and launch plan generation.
                        </p>
                    ) : (
                        <div className="space-y-3">
                            <p className="text-xs text-muted-foreground">
                                Spec status: <span className="font-medium text-foreground">{specIsApprovedForPlanning ? "approved" : "draft"}</span>
                            </p>
                            <p className="text-xs text-muted-foreground">
                                Active flow source for plan-generation launch: <span className="font-mono text-foreground">{activeFlow || "none selected"}</span>
                            </p>
                            <div className="flex flex-wrap items-center gap-2">
                                <button
                                    data-testid="project-spec-approve-for-plan-button"
                                    type="button"
                                    onClick={onApproveSpecForPlanning}
                                    disabled={!activeProjectScope?.specId}
                                    className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                >
                                    Approve spec for planning
                                </button>
                                <button
                                    data-testid="project-plan-generation-launch-button"
                                    type="button"
                                    onClick={() => {
                                        void onLaunchPlanGenerationWorkflow()
                                    }}
                                    disabled={!activeProjectScope?.specId || !specIsApprovedForPlanning || !activeFlow}
                                    className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                                >
                                    Launch plan-generation workflow
                                </button>
                            </div>
                            {!activeProjectScope?.specId ? (
                                <p className="text-[11px] text-muted-foreground">
                                    Create or open a project spec before approving and launching plan generation.
                                </p>
                            ) : null}
                            {activeProjectScope?.specId && !specIsApprovedForPlanning ? (
                                <p className="text-[11px] text-muted-foreground">
                                    Approve the active spec before launching the plan-generation workflow.
                                </p>
                            ) : null}
                            <div data-testid="project-plan-gate-surface" className="rounded-md border border-border px-3 py-2">
                                <p className="text-xs font-medium text-foreground">Plan gate controls</p>
                                <p className="text-[11px] text-muted-foreground">
                                    Plan status: <span className="font-medium text-foreground">{activePlanStatus}</span>
                                </p>
                                {!activeProjectScope?.planId ? (
                                    <p className="text-[11px] text-muted-foreground">
                                        Generate or open a plan artifact before applying plan gate actions.
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
                                <p data-testid="project-plan-generation-error" className="text-[11px] text-destructive">
                                    {planGenerationError}
                                </p>
                            ) : null}
                            {planGenerationStatusDegraded ? (
                                <p data-testid="project-plan-generation-status-degraded" className="text-[11px] text-amber-800">
                                    {planGenerationStatusDegraded}
                                </p>
                            ) : null}
                            {lastPlanGenerationFailure ? (
                                <div data-testid="project-plan-failure-diagnostics" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[11px] text-destructive">
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
                                                void onLaunchPlanGenerationWorkflow()
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
                                        <p className="text-[11px] text-destructive/90">
                                            Review the launch error, then rerun with the same project-scoped workflow inputs.
                                        </p>
                                    </div>
                                </div>
                            ) : null}
                        </div>
                    )}
                </div>
                <div className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <form data-testid="project-register-form" onSubmit={onSubmitProjectRegistration}>
                        <label htmlFor="project-path-input" className="mb-2 block text-xs font-medium text-foreground">
                            Project directory path
                        </label>
                        <div
                            data-testid="project-register-controls"
                            data-responsive-layout={isNarrowViewport ? "stacked" : "inline"}
                            className={`mb-3 ${isNarrowViewport ? "flex flex-col gap-2" : "flex gap-2"}`}
                        >
                            <input
                                id="project-path-input"
                                data-testid="project-path-input"
                                type="text"
                                value={directoryPathInput}
                                onChange={(event) => {
                                    setDirectoryPathInput(event.target.value)
                                    clearProjectRegistrationError()
                                }}
                                placeholder="/absolute/path/to/project"
                                className="h-9 flex-1 rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            />
                            <button
                                data-testid="project-register-button"
                                type="submit"
                                className="inline-flex h-9 items-center justify-center rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary/90 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            >
                                Register
                            </button>
                        </div>
                    </form>
                    {projectRegistrationError ? (
                        <p data-testid="project-registration-error" className="mb-3 text-sm text-destructive">
                            {projectRegistrationError}
                        </p>
                    ) : null}
                    <ul data-testid="project-registry-list" className="space-y-2">
                        {projects.length === 0 ? (
                            <li className="rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
                                No projects registered yet.
                            </li>
                        ) : (
                            projects.map((project) => {
                                const isActive = project.directoryPath === activeProjectPath
                                const isEditing = editingProjectPath === project.directoryPath
                                return (
                                    <li key={project.directoryPath} className="rounded-md border border-border px-3 py-2">
                                        {isEditing ? (
                                            <div className="space-y-2">
                                                <input
                                                    data-testid="project-edit-input"
                                                    aria-label="Edit project directory path"
                                                    type="text"
                                                    value={editingDirectoryPathInput}
                                                    onChange={(event) => {
                                                        setEditingDirectoryPathInput(event.target.value)
                                                        clearProjectRegistrationError()
                                                    }}
                                                    className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                                <div className="flex items-center justify-end gap-2">
                                                    <button
                                                        data-testid="project-edit-cancel-button"
                                                        type="button"
                                                        onClick={onCancelProjectPathEdit}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        Cancel
                                                    </button>
                                                    <button
                                                        data-testid="project-edit-save-button"
                                                        type="button"
                                                        onClick={() => {
                                                            const result = updateProjectPath(project.directoryPath, editingDirectoryPathInput)
                                                            if (result.ok) {
                                                                setEditingProjectPath(null)
                                                                setEditingDirectoryPathInput("")
                                                            }
                                                        }}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        Save
                                                    </button>
                                                </div>
                                            </div>
                                        ) : (
                                            <div
                                                className={
                                                    isNarrowViewport
                                                        ? "flex flex-col items-start gap-3"
                                                        : "flex items-center justify-between gap-3"
                                                }
                                            >
                                                <div className="min-w-0 flex-1 space-y-1">
                                                    {(() => {
                                                        const projectName = project.directoryPath.split('/').filter(Boolean).pop() || project.directoryPath
                                                        const branchLabel =
                                                            projectGitMetadata[project.directoryPath]?.branch || "Unknown branch"
                                                        const lastActivityLabel = formatLastActivity(project.lastAccessedAt)
                                                        return (
                                                            <>
                                                                <p data-testid="project-metadata-name" className="truncate text-sm font-medium text-foreground">
                                                                    Name: {projectName}
                                                                </p>
                                                                <p data-testid="project-metadata-directory" className="truncate text-xs text-muted-foreground">
                                                                    Directory: {project.directoryPath}
                                                                </p>
                                                                <p data-testid="project-metadata-branch" className="truncate text-xs text-muted-foreground">
                                                                    Branch: {branchLabel}
                                                                </p>
                                                                <p data-testid="project-metadata-last-activity" className="truncate text-xs text-muted-foreground">
                                                                    Last activity: {lastActivityLabel}
                                                                </p>
                                                            </>
                                                        )
                                                    })()}
                                                </div>
                                                <div
                                                    data-testid="project-row-actions"
                                                    data-responsive-layout={isNarrowViewport ? "stacked" : "inline"}
                                                    className={
                                                        isNarrowViewport
                                                            ? "flex w-full flex-col items-stretch gap-2"
                                                            : "flex items-center gap-2"
                                                    }
                                                >
                                                    <button
                                                        data-testid="favorite-toggle-button"
                                                        type="button"
                                                        onClick={() => toggleProjectFavorite(project.directoryPath)}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        {project.isFavorite ? "Unfavorite" : "Favorite"}
                                                    </button>
                                                    <button
                                                        type="button"
                                                        onClick={() => setActiveProjectPath(project.directoryPath)}
                                                        aria-current={isActive ? "true" : undefined}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        {isActive ? "Active" : "Set active"}
                                                    </button>
                                                    <button
                                                        data-testid="project-edit-button"
                                                        type="button"
                                                        onClick={() => onStartProjectPathEdit(project.directoryPath)}
                                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    >
                                                        Update path
                                                    </button>
                                                </div>
                                            </div>
                                        )}
                                    </li>
                                )
                            })
                        )}
                    </ul>
                </div>
            </div>
        </section>
    )
}
