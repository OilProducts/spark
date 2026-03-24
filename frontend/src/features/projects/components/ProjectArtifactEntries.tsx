import { ChevronDown, ChevronUp } from "lucide-react"

import type { ExecutionCardResponse, FlowLaunchResponse, FlowRunRequestResponse, SpecEditProposalResponse } from "@/lib/workspaceClient"

import type { ProjectGitMetadata } from "@/components/projects/presentation"

type SurfaceTone = "neutral" | "info" | "success" | "warning" | "danger"

type ProposalDiffLine = {
    type: "removed" | "added"
    text: string
}

type ProjectSpecEditProposalEntryProps = {
    proposal: SpecEditProposalResponse | null
    activeProjectGitMetadata: ProjectGitMetadata
    isLatestProposal: boolean
    pendingSpecProposalId: string | null
    expandedProposalChanges: Record<string, boolean>
    onApproveSpecEditProposal: (proposal: SpecEditProposalResponse) => void | Promise<void>
    onRejectSpecEditProposal: (proposal: SpecEditProposalResponse) => void | Promise<void>
    toggleProposalChangeExpanded: (changeKey: string) => void
    formatConversationTimestamp: (value: string) => string
    getSpecEditStatusPresentation: (status: SpecEditProposalResponse["status"]) => { label: string; tone: SurfaceTone }
    getSurfaceToneClassName: (tone: SurfaceTone) => string
    buildProposalDiffLines: (change: SpecEditProposalResponse["changes"][number]) => ProposalDiffLine[]
    buildProposalChangeKey: (proposalId: string, path: string, changeIndex: number) => string
    proposalDiffCollapseLineLimit: number
}

export function ProjectSpecEditProposalEntry({
    proposal,
    activeProjectGitMetadata,
    isLatestProposal,
    pendingSpecProposalId,
    expandedProposalChanges,
    onApproveSpecEditProposal,
    onRejectSpecEditProposal,
    toggleProposalChangeExpanded,
    formatConversationTimestamp,
    getSpecEditStatusPresentation,
    getSurfaceToneClassName,
    buildProposalDiffLines,
    buildProposalChangeKey,
    proposalDiffCollapseLineLimit,
}: ProjectSpecEditProposalEntryProps) {
    if (!proposal) {
        return (
            <div className="w-full rounded-md border border-border bg-muted/40 px-4 py-3 text-xs text-muted-foreground">
                Spec edit artifact unavailable. Refresh the project chat to reload it.
            </div>
        )
    }

    const statusPresentation = getSpecEditStatusPresentation(proposal.status)
    const proposalBranch = proposal.git_branch ?? activeProjectGitMetadata.branch
    const proposalCommit = proposal.git_commit ?? activeProjectGitMetadata.commit

    return (
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
                    const shouldCollapse = diffLines.length > proposalDiffCollapseLineLimit
                    const changeKey = buildProposalChangeKey(proposal.id, change.path, changeIndex)
                    const isExpanded = expandedProposalChanges[changeKey] === true
                    const visibleLines = shouldCollapse && !isExpanded
                        ? diffLines.slice(0, proposalDiffCollapseLineLimit)
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
                                        Showing first {proposalDiffCollapseLineLimit} of {diffLines.length} lines.
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
    )
}

type ProjectFlowRunRequestEntryProps = {
    flowRunRequest: FlowRunRequestResponse | null
    isLatestFlowRunRequest: boolean
    pendingFlowRunRequestId: string | null
    onReviewFlowRunRequest: (request: FlowRunRequestResponse, disposition: "approved" | "rejected") => void | Promise<void>
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    formatConversationTimestamp: (value: string) => string
    getFlowRunRequestStatusPresentation: (status: FlowRunRequestResponse["status"]) => { label: string; tone: SurfaceTone }
    getSurfaceToneClassName: (tone: SurfaceTone) => string
}

export function ProjectFlowRunRequestEntry({
    flowRunRequest,
    isLatestFlowRunRequest,
    pendingFlowRunRequestId,
    onReviewFlowRunRequest,
    onOpenFlowRun,
    formatConversationTimestamp,
    getFlowRunRequestStatusPresentation,
    getSurfaceToneClassName,
}: ProjectFlowRunRequestEntryProps) {
    if (!flowRunRequest) {
        return (
            <div className="w-full rounded-md border border-border bg-muted/40 px-4 py-3 text-xs text-muted-foreground">
                Flow run request artifact unavailable. Refresh the project chat to reload it.
            </div>
        )
    }

    const statusPresentation = getFlowRunRequestStatusPresentation(flowRunRequest.status)
    const canReview = flowRunRequest.status === "pending" || flowRunRequest.status === "launch_failed"
    const launchContextText = flowRunRequest.launch_context
        ? JSON.stringify(flowRunRequest.launch_context, null, 2)
        : null

    return (
        <div
            data-testid={isLatestFlowRunRequest ? "project-flow-run-request-surface" : undefined}
            className="w-full rounded-md border border-emerald-500/20 bg-emerald-500/[0.05] px-4 py-3"
        >
            <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                        <p className="text-[10px] font-semibold uppercase tracking-wide text-emerald-700">
                            Flow run request
                        </p>
                        <span className={getSurfaceToneClassName(statusPresentation.tone)}>
                            {statusPresentation.label}
                        </span>
                    </div>
                    <p className="text-sm font-medium text-foreground">{flowRunRequest.summary}</p>
                </div>
                <div className="space-y-1 text-right text-[11px] text-muted-foreground">
                    <p className="font-mono text-foreground">{flowRunRequest.id}</p>
                    <p>Updated {formatConversationTimestamp(flowRunRequest.updated_at)}</p>
                </div>
            </div>
            <div className="mt-3 space-y-2 text-[11px] text-muted-foreground">
                <p>
                    Flow: <span className="font-mono text-foreground">{flowRunRequest.flow_name}</span>
                </p>
                {flowRunRequest.goal ? (
                    <p className="whitespace-pre-wrap rounded border border-border/60 bg-background/80 px-2 py-1 text-[11px] text-muted-foreground">
                        {flowRunRequest.goal}
                    </p>
                ) : null}
                {launchContextText ? (
                    <div className="space-y-1">
                        <p>
                            Launch context:
                        </p>
                        <pre className="overflow-x-auto whitespace-pre-wrap rounded border border-border/60 bg-background/80 px-2 py-2 font-mono text-[10px] text-muted-foreground">
                            {launchContextText}
                        </pre>
                    </div>
                ) : null}
                {flowRunRequest.model ? (
                    <p>
                        Model override: <span className="font-mono text-foreground">{flowRunRequest.model}</span>
                    </p>
                ) : null}
                {flowRunRequest.review_message ? (
                    <p>
                        Review note: <span className="text-foreground">{flowRunRequest.review_message}</span>
                    </p>
                ) : null}
                {flowRunRequest.launch_error ? (
                    <p className="text-destructive">
                        Launch error: {flowRunRequest.launch_error}
                    </p>
                ) : null}
                {flowRunRequest.run_id ? (
                    <div className="flex flex-wrap items-center gap-2">
                        <span>
                            Run: <span className="font-mono text-foreground">{flowRunRequest.run_id}</span>
                        </span>
                        <button
                            type="button"
                            data-testid={isLatestFlowRunRequest ? "project-flow-run-request-open-run-button" : undefined}
                            onClick={() => onOpenFlowRun(flowRunRequest)}
                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        >
                            Open run
                        </button>
                    </div>
                ) : null}
            </div>
            {canReview ? (
                <div className="mt-3 flex flex-wrap items-center gap-2">
                    <button
                        data-testid={isLatestFlowRunRequest ? "project-flow-run-request-approve-button" : undefined}
                        type="button"
                        onClick={() => {
                            void onReviewFlowRunRequest(flowRunRequest, "approved")
                        }}
                        disabled={pendingFlowRunRequestId === flowRunRequest.id}
                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                    >
                        Approve run
                    </button>
                    <button
                        data-testid={isLatestFlowRunRequest ? "project-flow-run-request-reject-button" : undefined}
                        type="button"
                        onClick={() => {
                            void onReviewFlowRunRequest(flowRunRequest, "rejected")
                        }}
                        disabled={pendingFlowRunRequestId === flowRunRequest.id}
                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                    >
                        Reject run
                    </button>
                </div>
            ) : null}
        </div>
    )
}

type ProjectFlowLaunchEntryProps = {
    flowLaunch: FlowLaunchResponse | null
    isLatestFlowLaunch: boolean
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    formatConversationTimestamp: (value: string) => string
    getFlowLaunchStatusPresentation: (status: FlowLaunchResponse["status"]) => { label: string; tone: SurfaceTone }
    getSurfaceToneClassName: (tone: SurfaceTone) => string
}

export function ProjectFlowLaunchEntry({
    flowLaunch,
    isLatestFlowLaunch,
    onOpenFlowRun,
    formatConversationTimestamp,
    getFlowLaunchStatusPresentation,
    getSurfaceToneClassName,
}: ProjectFlowLaunchEntryProps) {
    if (!flowLaunch) {
        return (
            <div className="w-full rounded-md border border-border bg-muted/40 px-4 py-3 text-xs text-muted-foreground">
                Flow launch artifact unavailable. Refresh the project chat to reload it.
            </div>
        )
    }

    const statusPresentation = getFlowLaunchStatusPresentation(flowLaunch.status)
    const launchContextText = flowLaunch.launch_context
        ? JSON.stringify(flowLaunch.launch_context, null, 2)
        : null

    return (
        <div
            data-testid={isLatestFlowLaunch ? "project-flow-launch-surface" : undefined}
            className="w-full rounded-md border border-sky-500/20 bg-sky-500/[0.05] px-4 py-3"
        >
            <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                        <p className="text-[10px] font-semibold uppercase tracking-wide text-sky-700">
                            Flow launch
                        </p>
                        <span className={getSurfaceToneClassName(statusPresentation.tone)}>
                            {statusPresentation.label}
                        </span>
                    </div>
                    <p className="text-sm font-medium text-foreground">{flowLaunch.summary}</p>
                </div>
                <div className="space-y-1 text-right text-[11px] text-muted-foreground">
                    <p className="font-mono text-foreground">{flowLaunch.id}</p>
                    <p>Updated {formatConversationTimestamp(flowLaunch.updated_at)}</p>
                </div>
            </div>
            <div className="mt-3 space-y-2 text-[11px] text-muted-foreground">
                <p>
                    Flow: <span className="font-mono text-foreground">{flowLaunch.flow_name}</span>
                </p>
                {flowLaunch.goal ? (
                    <p className="whitespace-pre-wrap rounded border border-border/60 bg-background/80 px-2 py-1 text-[11px] text-muted-foreground">
                        {flowLaunch.goal}
                    </p>
                ) : null}
                {launchContextText ? (
                    <div className="space-y-1">
                        <p>Launch context:</p>
                        <pre className="overflow-x-auto whitespace-pre-wrap rounded border border-border/60 bg-background/80 px-2 py-2 font-mono text-[10px] text-muted-foreground">
                            {launchContextText}
                        </pre>
                    </div>
                ) : null}
                {flowLaunch.model ? (
                    <p>
                        Model override: <span className="font-mono text-foreground">{flowLaunch.model}</span>
                    </p>
                ) : null}
                {flowLaunch.launch_error ? (
                    <p className="text-destructive">
                        Launch error: {flowLaunch.launch_error}
                    </p>
                ) : null}
                {flowLaunch.run_id ? (
                    <div className="flex flex-wrap items-center gap-2">
                        <span>
                            Run: <span className="font-mono text-foreground">{flowLaunch.run_id}</span>
                        </span>
                        <button
                            type="button"
                            data-testid={isLatestFlowLaunch ? "project-flow-launch-open-run-button" : undefined}
                            onClick={() => onOpenFlowRun(flowLaunch)}
                            className="rounded border border-border px-2 py-1 text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                        >
                            Open run
                        </button>
                    </div>
                ) : null}
            </div>
        </div>
    )
}

type ProjectExecutionCardEntryProps = {
    executionCard: ExecutionCardResponse | null
    isLatestExecutionCard: boolean
    pendingExecutionCardId: string | null
    onReviewExecutionCard: (card: ExecutionCardResponse, disposition: "approved" | "rejected" | "revision_requested") => void | Promise<void>
    formatConversationTimestamp: (value: string) => string
    getExecutionCardStatusPresentation: (status: ExecutionCardResponse["status"]) => { label: string; tone: SurfaceTone }
    getSurfaceToneClassName: (tone: SurfaceTone) => string
}

export function ProjectExecutionCardEntry({
    executionCard,
    isLatestExecutionCard,
    pendingExecutionCardId,
    onReviewExecutionCard,
    formatConversationTimestamp,
    getExecutionCardStatusPresentation,
    getSurfaceToneClassName,
}: ProjectExecutionCardEntryProps) {
    if (!executionCard) {
        return (
            <div className="w-full rounded-md border border-border bg-muted/40 px-4 py-3 text-xs text-muted-foreground">
                Execution card artifact unavailable. Refresh the project chat to reload it.
            </div>
        )
    }

    const statusPresentation = getExecutionCardStatusPresentation(executionCard.status)
    const canReview = executionCard.status === "draft"

    return (
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
    )
}
