import { useState } from "react"
import { useStore } from "@/store"
import { buildPipelineStartPayload } from "@/lib/pipelineStartPayload"
import { Play, Settings2 } from "lucide-react"

export function Navbar() {
    const { viewMode, setViewMode, activeFlow, setSelectedRunId } = useStore()
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const model = useStore((state) => state.model)
    const workingDir = useStore((state) => state.workingDir)
    const diagnostics = useStore((state) => state.diagnostics)
    const hasValidationErrors = useStore((state) => state.hasValidationErrors)
    const hasValidationWarnings = diagnostics.some((diag) => diag.severity === 'warning')
    const showValidationWarningBanner = hasValidationWarnings && !hasValidationErrors
    const [runStartError, setRunStartError] = useState<string | null>(null)
    const runtimeStatus = useStore((state) => state.runtimeStatus)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const runInitiationForm = {
        projectPath: activeProjectPath || '',
        flowSource: activeFlow || '',
        workingDirectory: workingDir,
        backend: 'codex',
        model: model.trim() || null,
    }

    const runPipeline = async () => {
        if (!activeProjectPath || !activeFlow || hasValidationErrors) return

        setRunStartError(null)
        try {
            const flowRes = await fetch(`/api/flows/${encodeURIComponent(runInitiationForm.flowSource)}`)
            if (!flowRes.ok) {
                throw new Error(`Failed to load flow: ${runInitiationForm.flowSource}`)
            }

            const flow = await flowRes.json()
            const startPayload = buildPipelineStartPayload(runInitiationForm, flow.content)
            const runRes = await fetch('/pipelines', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(startPayload),
            })
            if (!runRes.ok) {
                let reason = `Run request failed (${runRes.status})`
                try {
                    const detail = await runRes.json()
                    if (detail && typeof detail === 'object') {
                        if (typeof detail.error === 'string' && detail.error.trim()) {
                            reason = detail.error.trim()
                        } else if (typeof detail.detail === 'string' && detail.detail.trim()) {
                            reason = detail.detail.trim()
                        }
                    }
                } catch {
                    // Keep the HTTP-based fallback reason when error details are unavailable.
                }
                throw new Error(reason)
            }

            const runData = await runRes.json()
            if (runData?.status !== 'started') {
                const reason = runData?.error || runData?.status || 'Unknown run error'
                throw new Error(`Run not started: ${reason}`)
            }
            if (typeof runData?.pipeline_id === 'string') {
                setSelectedRunId(runData.pipeline_id)
            }

            setViewMode('execution')
        } catch (error) {
            console.error(error)
            setRunStartError(error instanceof Error ? error.message : 'Failed to start pipeline run.')
        }
    }

    const projectLabel = activeProjectPath || "No active project"
    const flowLabel = activeFlow || "No active flow"
    const runContextLabel = selectedRunId
        ? `${runtimeStatus} · ${selectedRunId}`
        : `${runtimeStatus} · no run selected`

    return (
        <header data-testid="top-nav" className="h-14 border-b bg-background flex items-center justify-between px-6 shrink-0 z-50">
            <div className="flex items-center gap-8">
                <div className="flex items-center gap-2">
                    <Settings2 className="w-5 h-5" />
                    <span className="font-semibold tracking-tight">Attractor React</span>
                </div>

                <div data-testid="view-mode-tabs" className="inline-flex h-10 items-center justify-center rounded-md bg-muted p-1 text-muted-foreground w-[480px]">
                    <button
                        data-testid="nav-mode-projects"
                        onClick={() => setViewMode('projects')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'projects' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Projects
                    </button>
                    <button
                        data-testid="nav-mode-editor"
                        onClick={() => setViewMode('editor')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'editor' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Editor
                    </button>
                    <button
                        data-testid="nav-mode-execution"
                        onClick={() => setViewMode('execution')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'execution' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Execution
                    </button>
                    <button
                        data-testid="nav-mode-settings"
                        onClick={() => setViewMode('settings')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'settings' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Settings
                    </button>
                    <button
                        data-testid="nav-mode-runs"
                        onClick={() => setViewMode('runs')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'runs' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Runs
                    </button>
                </div>

                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <div data-testid="top-nav-active-project" className="max-w-56 truncate rounded border border-border bg-muted/40 px-2 py-1">
                        <span className="font-medium text-foreground">Project:</span> {projectLabel}
                    </div>
                    <div data-testid="top-nav-active-flow" className="max-w-48 truncate rounded border border-border bg-muted/40 px-2 py-1">
                        <span className="font-medium text-foreground">Flow:</span> {flowLabel}
                    </div>
                    <div data-testid="top-nav-run-context" className="max-w-56 truncate rounded border border-border bg-muted/40 px-2 py-1">
                        <span className="font-medium text-foreground">Runtime:</span> {runContextLabel}
                    </div>
                </div>
            </div>

            <div className="flex items-center gap-2">
                <div
                    data-testid="run-initiation-form"
                    className="hidden max-w-[560px] items-center gap-2 truncate rounded-md border border-border bg-muted/40 px-2 py-1 text-[11px] text-muted-foreground xl:flex"
                >
                    <span data-testid="run-initiation-project" className="truncate">
                        <span className="font-medium text-foreground">Project:</span> {runInitiationForm.projectPath || 'none'}
                    </span>
                    <span className="text-muted-foreground/70">|</span>
                    <span data-testid="run-initiation-flow-source" className="truncate">
                        <span className="font-medium text-foreground">Flow Source:</span> {runInitiationForm.flowSource || 'none'}
                    </span>
                    <span className="text-muted-foreground/70">|</span>
                    <span data-testid="run-initiation-working-directory" className="truncate font-mono">
                        <span className="font-medium text-foreground not-italic">WD:</span> {runInitiationForm.workingDirectory}
                    </span>
                    <span className="text-muted-foreground/70">|</span>
                    <span data-testid="run-initiation-backend-model" className="truncate">
                        <span className="font-medium text-foreground">Backend/Model:</span> {runInitiationForm.backend} / {runInitiationForm.model || 'default'}
                    </span>
                </div>
                {showValidationWarningBanner ? (
                    <p
                        data-testid="execute-warning-banner"
                        className="rounded border border-amber-400 bg-amber-50 px-2 py-1 text-[11px] font-medium leading-none text-amber-900"
                    >
                        Warnings present; run allowed.
                    </p>
                ) : null}
                {runStartError ? (
                    <p
                        data-testid="run-start-error-banner"
                        className="max-w-sm truncate rounded border border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] font-medium leading-none text-destructive"
                    >
                        Failed to start run: {runStartError}
                    </p>
                ) : null}
                <button
                    data-testid="execute-button"
                    onClick={runPipeline}
                    disabled={!activeProjectPath || !activeFlow || hasValidationErrors}
                    title={
                        !activeProjectPath
                            ? 'Select an active project before running.'
                            : hasValidationErrors
                                ? 'Fix validation errors before running.'
                                : showValidationWarningBanner
                                    ? 'Warnings are present. Review diagnostics before running.'
                                : undefined
                    }
                    className="inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0 bg-primary text-primary-foreground shadow hover:bg-primary/90 h-9 px-4 py-2"
                >
                    <Play className="w-4 h-4" />
                    Execute
                </button>
            </div>
        </header>
    )
}
