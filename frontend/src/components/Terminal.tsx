import { useStore } from "@/store"
import { ApiHttpError, fetchPipelineGraphValidated } from '@/lib/apiClient'
import { TerminalSquare } from "lucide-react"
import { useEffect, useMemo, useRef, useState } from "react"
import { ExplainabilityPanel } from "./ExplainabilityPanel"

type GraphArtifactAvailability = "idle" | "checking" | "available" | "missing" | "error"

const ACTIVE_RUNTIME_STATUSES = new Set(["running", "cancel_requested", "abort_requested"])

export function Terminal() {
    const viewMode = useStore((state) => state.viewMode)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const runtimeStatus = useStore((state) => state.runtimeStatus)
    const logs = useStore((state) => state.logs)
    const clearLogs = useStore((state) => state.clearLogs)
    const [graphArtifactAvailability, setGraphArtifactAvailability] = useState<GraphArtifactAvailability>("idle")
    const logsEndRef = useRef<HTMLDivElement>(null)

    useEffect(() => {
        logsEndRef.current?.scrollIntoView({ behavior: 'smooth' })
    }, [logs])

    useEffect(() => {
        if (!selectedRunId) {
            setGraphArtifactAvailability("idle")
            return
        }

        let isCancelled = false
        const probeGraphArtifact = async () => {
            setGraphArtifactAvailability((current) => (current === "available" ? current : "checking"))
            try {
                if (isCancelled) {
                    return
                }
                await fetchPipelineGraphValidated(selectedRunId)
                setGraphArtifactAvailability("available")
            } catch (error) {
                if (error instanceof ApiHttpError && error.status === 404) {
                    setGraphArtifactAvailability("missing")
                    return
                }
                if (!isCancelled) {
                    setGraphArtifactAvailability("error")
                }
            }
        }

        void probeGraphArtifact()
        if (!ACTIVE_RUNTIME_STATUSES.has(runtimeStatus)) {
            return () => {
                isCancelled = true
            }
        }

        const interval = window.setInterval(() => {
            void probeGraphArtifact()
        }, 5000)

        return () => {
            isCancelled = true
            window.clearInterval(interval)
        }
    }, [selectedRunId, runtimeStatus])

    const graphArtifactStatusLabel = useMemo(() => {
        if (!selectedRunId) {
            return "Select a planning/build run to inspect artifacts."
        }
        if (graphArtifactAvailability === "available") {
            return "Graph artifact is available."
        }
        if (graphArtifactAvailability === "checking") {
            return "Checking graph artifact availability..."
        }
        if (graphArtifactAvailability === "missing") {
            return "Graph artifact not available for this run yet."
        }
        if (graphArtifactAvailability === "error") {
            return "Could not load graph artifact availability."
        }
        return "Graph artifact status unknown."
    }, [selectedRunId, graphArtifactAvailability])
    const latestLog = logs.length > 0 ? logs[logs.length - 1] : null

    if (viewMode !== 'execution') return null

    return (
        <footer
            data-testid="execution-footer-stream"
            className="h-72 border-t bg-background flex flex-col z-30 shadow-[0_-10px_40px_rgba(0,0,0,0.1)]"
        >
            <div className="h-10 border-b flex items-center justify-between px-4 bg-muted/30 shrink-0">
                <div className="flex items-center gap-2">
                    <TerminalSquare className="w-4 h-4 text-muted-foreground" />
                    <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Terminal Output</span>
                </div>
                <div className="flex items-center gap-2">
                    <span data-testid="execution-footer-workflow-status" className="rounded border border-border bg-background px-2 py-0.5 text-[11px] text-muted-foreground">
                        Status: {runtimeStatus}
                    </span>
                    <span data-testid="execution-footer-log-count" className="rounded border border-border bg-background px-2 py-0.5 text-[11px] text-muted-foreground">
                        Logs: {logs.length}
                    </span>
                    <button
                        data-testid="execution-footer-terminal-clear"
                        onClick={clearLogs}
                        className="h-7 px-2 text-xs font-medium text-muted-foreground hover:text-foreground"
                    >
                        Clear
                    </button>
                </div>
            </div>
            <ExplainabilityPanel />
            <div data-testid="execution-footer-workflow-artifacts" className="flex items-center justify-between gap-3 border-b bg-muted/10 px-4 py-2 text-[11px]">
                <div className="min-w-0">
                    <p data-testid="execution-footer-workflow-artifact-status" className="truncate text-muted-foreground">
                        {graphArtifactStatusLabel}
                    </p>
                    <p className="truncate text-muted-foreground/80">
                        {latestLog ? `Latest log: ${latestLog.msg}` : "No runtime logs yet."}
                    </p>
                </div>
                {selectedRunId && graphArtifactAvailability === "available" ? (
                    <a
                        data-testid="execution-footer-workflow-artifact-link"
                        href={`/pipelines/${encodeURIComponent(selectedRunId)}/graph`}
                        target="_blank"
                        rel="noreferrer"
                        className="whitespace-nowrap rounded border border-border px-2 py-1 text-[11px] font-medium text-foreground hover:bg-muted"
                    >
                        Open graph artifact
                    </a>
                ) : null}
            </div>
            <div data-testid="execution-footer-terminal-output" className="flex-1 p-4 overflow-y-auto font-mono text-sm space-y-1">
                {logs.map((log, i) => (
                    <div key={i} className="flex gap-4 py-0.5 border-b border-border/50 last:border-0 hover:bg-muted/50 rounded px-2">
                        <span className="text-muted-foreground w-20 shrink-0 select-none">{log.time}</span>
                        <span className={`break-all ${log.type === 'success' ? 'text-green-500' :
                            log.type === 'error' ? 'text-destructive' :
                                'text-foreground'
                            }`}>
                            {log.msg}
                        </span>
                    </div>
                ))}
                <div ref={logsEndRef} />
            </div>
        </footer>
    )
}
