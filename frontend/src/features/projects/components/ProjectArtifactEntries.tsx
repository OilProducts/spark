import { Button } from "@/components/ui/button"
import { usePipelineResultPreview } from "../hooks/usePipelineResultPreview"
import type {
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
} from "../model/types"
import { ProjectConversationMarkdown } from "./ProjectConversationMarkdown"

type SurfaceTone = "neutral" | "info" | "success" | "warning" | "danger"

type ProjectFlowRunRequestEntryProps = {
    flowRunRequest: ProjectFlowRunRequest | null
    isLatestFlowRunRequest: boolean
    pendingFlowRunRequestId: string | null
    onReviewFlowRunRequest: (request: ProjectFlowRunRequest, disposition: "approved" | "rejected") => void | Promise<void>
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    formatConversationTimestamp: (value: string) => string
    getFlowRunRequestStatusPresentation: (status: ProjectFlowRunRequest["status"]) => { label: string; tone: SurfaceTone }
    getSurfaceToneClassName: (tone: SurfaceTone) => string
}

function ProjectFlowResultPreview({ runId }: { runId: string }) {
    const { result, isLoading, error, viewResult } = usePipelineResultPreview(runId)

    return (
        <div className="space-y-2">
            <Button
                type="button"
                data-testid="project-flow-launch-view-result-button"
                onClick={() => {
                    void viewResult()
                }}
                variant="outline"
                size="xs"
                className="px-2 text-xs"
            >
                {isLoading ? "Loading result..." : "View result"}
            </Button>
            {error ? <p className="text-destructive">{error}</p> : null}
            {result?.state === "pending" ? (
                <p className="text-muted-foreground">Result pending.</p>
            ) : null}
            {result?.state === "unavailable" ? (
                <p className="text-muted-foreground">Result unavailable.</p>
            ) : null}
            {result?.state === "error" ? (
                <p className="text-destructive">{result.error || "Result resolution failed."}</p>
            ) : null}
            {result?.state === "ready" ? (
                <div data-testid="project-flow-launch-result-body" className="rounded border border-border/60 bg-background/80 px-2 py-2">
                    <ProjectConversationMarkdown content={result.body_markdown} />
                </div>
            ) : null}
        </div>
    )
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
                {flowRunRequest.llm_provider ? (
                    <p>
                        Provider override: <span className="font-mono text-foreground">{flowRunRequest.llm_provider}</span>
                    </p>
                ) : null}
                {flowRunRequest.reasoning_effort ? (
                    <p>
                        Reasoning effort: <span className="font-mono text-foreground">{flowRunRequest.reasoning_effort}</span>
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
                        <Button
                            type="button"
                            data-testid={isLatestFlowRunRequest ? "project-flow-run-request-open-run-button" : undefined}
                            onClick={() => onOpenFlowRun(flowRunRequest)}
                            variant="outline"
                            size="xs"
                            className="px-2 text-xs"
                        >
                            Open run
                        </Button>
                        <ProjectFlowResultPreview runId={flowRunRequest.run_id} />
                    </div>
                ) : null}
            </div>
            {canReview ? (
                <div className="mt-3 flex flex-wrap items-center gap-2">
                    <Button
                        data-testid={isLatestFlowRunRequest ? "project-flow-run-request-approve-button" : undefined}
                        type="button"
                        onClick={() => {
                            void onReviewFlowRunRequest(flowRunRequest, "approved")
                        }}
                        disabled={pendingFlowRunRequestId === flowRunRequest.id}
                        variant="outline"
                        size="xs"
                        className="px-2 text-xs"
                    >
                        Approve run
                    </Button>
                    <Button
                        data-testid={isLatestFlowRunRequest ? "project-flow-run-request-reject-button" : undefined}
                        type="button"
                        onClick={() => {
                            void onReviewFlowRunRequest(flowRunRequest, "rejected")
                        }}
                        disabled={pendingFlowRunRequestId === flowRunRequest.id}
                        variant="outline"
                        size="xs"
                        className="px-2 text-xs"
                    >
                        Reject run
                    </Button>
                </div>
            ) : null}
        </div>
    )
}

type ProjectFlowLaunchEntryProps = {
    flowLaunch: ProjectFlowLaunch | null
    isLatestFlowLaunch: boolean
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    formatConversationTimestamp: (value: string) => string
    getFlowLaunchStatusPresentation: (status: ProjectFlowLaunch["status"]) => { label: string; tone: SurfaceTone }
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
                {flowLaunch.llm_provider ? (
                    <p>
                        Provider override: <span className="font-mono text-foreground">{flowLaunch.llm_provider}</span>
                    </p>
                ) : null}
                {flowLaunch.reasoning_effort ? (
                    <p>
                        Reasoning effort: <span className="font-mono text-foreground">{flowLaunch.reasoning_effort}</span>
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
                        <Button
                            type="button"
                            data-testid={isLatestFlowLaunch ? "project-flow-launch-open-run-button" : undefined}
                            onClick={() => onOpenFlowRun(flowLaunch)}
                            variant="outline"
                            size="xs"
                            className="px-2 text-xs"
                        >
                            Open run
                        </Button>
                        <ProjectFlowResultPreview runId={flowLaunch.run_id} />
                    </div>
                ) : null}
            </div>
        </div>
    )
}
