import { useStore } from "@/store"
import { Play, Settings2 } from "lucide-react"

export function Navbar() {
    const { viewMode, setViewMode, activeFlow, setSelectedRunId } = useStore()
    const model = useStore((state) => state.model)
    const workingDir = useStore((state) => state.workingDir)
    const hasValidationErrors = useStore((state) => state.hasValidationErrors)

    const runPipeline = async () => {
        if (!activeFlow || hasValidationErrors) return

        try {
            const flowRes = await fetch(`/api/flows/${encodeURIComponent(activeFlow)}`)
            if (!flowRes.ok) {
                throw new Error(`Failed to load flow: ${activeFlow}`)
            }

            const flow = await flowRes.json()
            const runRes = await fetch('/pipelines', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    flow_content: flow.content,
                    working_directory: workingDir,
                    backend: 'codex',
                    model: model.trim() || null,
                    flow_name: activeFlow,
                }),
            })
            if (!runRes.ok) {
                throw new Error('Run request failed')
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
            window.alert('Failed to start pipeline run. Check backend logs for details.')
        }
    }

    return (
        <header className="h-14 border-b bg-background flex items-center justify-between px-6 shrink-0 z-50">
            <div className="flex items-center gap-8">
                <div className="flex items-center gap-2">
                    <Settings2 className="w-5 h-5" />
                    <span className="font-semibold tracking-tight">Attractor React</span>
                </div>

                <div className="inline-flex h-10 items-center justify-center rounded-md bg-muted p-1 text-muted-foreground w-[380px]">
                    <button
                        onClick={() => setViewMode('editor')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'editor' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Editor
                    </button>
                    <button
                        onClick={() => setViewMode('execution')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'execution' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Execution
                    </button>
                    <button
                        onClick={() => setViewMode('settings')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'settings' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Settings
                    </button>
                    <button
                        onClick={() => setViewMode('runs')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'runs' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Runs
                    </button>
                </div>
            </div>

            <div className="flex items-center gap-4">
                <button
                    onClick={runPipeline}
                    disabled={!activeFlow || hasValidationErrors}
                    title={hasValidationErrors ? 'Fix validation errors before running.' : undefined}
                    className="inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0 bg-primary text-primary-foreground shadow hover:bg-primary/90 h-9 px-4 py-2"
                >
                    <Play className="w-4 h-4" />
                    Execute
                </button>
            </div>
        </header>
    )
}
