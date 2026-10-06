import { useState, type ReactNode } from "react"
import { Button } from "@/components/ui/button"
import { RunCard } from "@/components/app/transcript/SegmentRows"
import type { StatusTone } from "@/components/app/status"
import { Well } from "@/components/app/well"
import { flowTitle } from "@/features/runs/model/runOverviewModel"
import { formatDuration, formatRunStatusLabel, runStatusTone } from "@/features/runs/model/shared"
import type { PipelineResultResponse } from "@/lib/attractorClient"
import { useStore } from "@/store"
import { loadPipelineResult } from "../services/conversationArtifacts"
import type {
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
} from "../model/types"
import { ProjectConversationMarkdown } from "./ProjectConversationMarkdown"

type ProjectFlowRunRequestEntryProps = {
    flowRunRequest: ProjectFlowRunRequest | null
    isLatestFlowRunRequest: boolean
    pendingFlowRunRequestId: string | null
    onReviewFlowRunRequest: (request: ProjectFlowRunRequest, disposition: "approved" | "rejected") => void | Promise<void>
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    formatConversationTimestamp: (value: string) => string
    getFlowRunRequestStatusPresentation: (status: ProjectFlowRunRequest["status"]) => { label: string; tone: StatusTone }
}

function ProjectFlowResultPreview({ runId }: { runId: string }) {
    const [result, setResult] = useState<PipelineResultResponse | null>(null)
    const [isLoading, setIsLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const viewResult = async () => {
        setIsLoading(true)
        setError(null)
        try {
            setResult(await loadPipelineResult(runId))
        } catch (err) {
            console.error(err)
            setResult(null)
            setError("Unable to load result.")
        } finally {
            setIsLoading(false)
        }
    }

    return (
        <div className="space-y-2">
            <Button
                type="button"
                data-testid="project-flow-launch-view-result-button"
                onClick={() => {
                    void viewResult()
                }}
                variant="ghost"
                size="xs"
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
                <div data-testid="project-flow-launch-result-body" className="rounded-lg bg-muted px-3 py-2">
                    <ProjectConversationMarkdown content={result.body_markdown} />
                </div>
            ) : null}
        </div>
    )
}

type FlowArtifact = Pick<
    ProjectFlowLaunch,
    "id" | "flow_name" | "summary" | "updated_at" | "goal" | "launch_context" | "model" | "llm_provider" | "reasoning_effort" | "run_id" | "launch_error"
>


/**
 * A flow run request or launch as a bordered card: the shared RunCard header (kicker,
 * title linking to the run, live status) over the artifact's details and actions.
 */
function FlowArtifactCard({
    artifact,
    kicker,
    presentation,
    testId,
    openRunTestId,
    onOpenFlowRun,
    formatConversationTimestamp,
    children,
}: {
    artifact: FlowArtifact
    kicker: string
    presentation: { label: string; tone: StatusTone }
    testId?: string
    openRunTestId?: string
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    formatConversationTimestamp: (value: string) => string
    children?: ReactNode
}) {
    // The run's own record, when the runs list has it, gives the card its live status.
    const run = useStore((state) => artifact.run_id ? state.runsListSession.runs.find((entry) => entry.run_id === artifact.run_id) : undefined)
    const launchContextText = artifact.launch_context ? JSON.stringify(artifact.launch_context, null, 2) : null
    return (
        <div data-testid={testId} className="w-full max-w-[72ch] rounded-xl border border-border">
            <RunCard
                className="rounded-none border-0 p-3"
                kicker={`${kicker} · ${flowTitle(artifact.flow_name)}`}
                title={artifact.run_id ? (
                    <button
                        type="button"
                        data-testid={openRunTestId}
                        onClick={() => onOpenFlowRun(artifact)}
                        className="max-w-full truncate text-left outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                    >
                        {artifact.summary}
                    </button>
                ) : artifact.summary}
                tone={run ? runStatusTone(run.status) : presentation.tone}
                status={run ? formatRunStatusLabel(run) : presentation.label}
                duration={run ? formatDuration(run.started_at, run.ended_at, run.status) : undefined}
            />
            <div className="space-y-2 px-3 pb-3 text-xs text-muted-foreground">
                <p>
                    Flow: <span className="font-mono text-foreground">{artifact.flow_name}</span>
                    {" · "}<span className="font-mono">{artifact.id}</span>
                    {" · "}Updated {formatConversationTimestamp(artifact.updated_at)}
                </p>
                {artifact.goal ? (
                    <p className="whitespace-pre-wrap rounded-lg bg-muted px-3 py-2 text-sm text-foreground">{artifact.goal}</p>
                ) : null}
                {launchContextText ? (
                    <div className="space-y-1">
                        <p>Launch context:</p>
                        <pre className="overflow-x-auto whitespace-pre-wrap rounded-lg bg-muted px-3 py-2 font-mono text-xs text-muted-foreground">
                            {launchContextText}
                        </pre>
                    </div>
                ) : null}
                {artifact.model ? <p>Model override: <span className="font-mono text-foreground">{artifact.model}</span></p> : null}
                {artifact.llm_provider ? <p>Provider override: <span className="font-mono text-foreground">{artifact.llm_provider}</span></p> : null}
                {artifact.reasoning_effort ? <p>Reasoning effort: <span className="font-mono text-foreground">{artifact.reasoning_effort}</span></p> : null}
                {children}
                {artifact.launch_error ? <p className="text-destructive">Launch error: {artifact.launch_error}</p> : null}
                {artifact.run_id ? (
                    <div className="flex flex-wrap items-center gap-2">
                        <span>Run: <span className="font-mono text-foreground">{artifact.run_id}</span></span>
                        <ProjectFlowResultPreview runId={artifact.run_id} />
                    </div>
                ) : null}
            </div>
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
}: ProjectFlowRunRequestEntryProps) {
    if (!flowRunRequest) {
        return (
            <Well className="w-full px-4 py-3 text-xs text-muted-foreground">
                Flow run request artifact unavailable. Refresh the project chat to reload it.
            </Well>
        )
    }

    const canReview = flowRunRequest.status === "pending" || flowRunRequest.status === "launch_failed"

    return (
        <FlowArtifactCard
            artifact={flowRunRequest}
            kicker="Run request"
            presentation={getFlowRunRequestStatusPresentation(flowRunRequest.status)}
            testId={isLatestFlowRunRequest ? "project-flow-run-request-surface" : undefined}
            openRunTestId={isLatestFlowRunRequest ? "project-flow-run-request-open-run-button" : undefined}
            onOpenFlowRun={onOpenFlowRun}
            formatConversationTimestamp={formatConversationTimestamp}
        >
            {flowRunRequest.review_message ? (
                <p>Review note: <span className="text-foreground">{flowRunRequest.review_message}</span></p>
            ) : null}
            {canReview ? (
                <div className="flex flex-wrap items-center gap-2 pt-1">
                    <Button
                        data-testid={isLatestFlowRunRequest ? "project-flow-run-request-approve-button" : undefined}
                        type="button"
                        onClick={() => {
                            void onReviewFlowRunRequest(flowRunRequest, "approved")
                        }}
                        disabled={pendingFlowRunRequestId === flowRunRequest.id}
                        variant="outline"
                        size="xs"
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
                        variant="ghost"
                        size="xs"
                    >
                        Reject run
                    </Button>
                </div>
            ) : null}
        </FlowArtifactCard>
    )
}

type ProjectFlowLaunchEntryProps = {
    flowLaunch: ProjectFlowLaunch | null
    isLatestFlowLaunch: boolean
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    formatConversationTimestamp: (value: string) => string
    getFlowLaunchStatusPresentation: (status: ProjectFlowLaunch["status"]) => { label: string; tone: StatusTone }
}

export function ProjectFlowLaunchEntry({
    flowLaunch,
    isLatestFlowLaunch,
    onOpenFlowRun,
    formatConversationTimestamp,
    getFlowLaunchStatusPresentation,
}: ProjectFlowLaunchEntryProps) {
    if (!flowLaunch) {
        return (
            <Well className="w-full px-4 py-3 text-xs text-muted-foreground">
                Flow launch artifact unavailable. Refresh the project chat to reload it.
            </Well>
        )
    }

    return (
        <FlowArtifactCard
            artifact={flowLaunch}
            kicker="Launched run"
            presentation={getFlowLaunchStatusPresentation(flowLaunch.status)}
            testId={isLatestFlowLaunch ? "project-flow-launch-surface" : undefined}
            openRunTestId={isLatestFlowLaunch ? "project-flow-launch-open-run-button" : undefined}
            onOpenFlowRun={onOpenFlowRun}
            formatConversationTimestamp={formatConversationTimestamp}
        />
    )
}
