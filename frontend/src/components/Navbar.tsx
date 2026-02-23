import { useState } from "react"
import { useStore } from "@/store"
import { Play, Settings2 } from "lucide-react"

export function Navbar() {
    const { viewMode, setViewMode, activeFlow } = useStore()
    const [model, setModel] = useState("")
    const [workingDir, setWorkingDir] = useState("./test-app")
    const [isRunning, setIsRunning] = useState(false)

    const runPipeline = async () => {
        if (!activeFlow || isRunning) return
        setIsRunning(true)

        try {
            const flowRes = await fetch(`/api/flows/${encodeURIComponent(activeFlow)}`)
            if (!flowRes.ok) {
                throw new Error(`Failed to load flow: ${activeFlow}`)
            }

            const flow = await flowRes.json()
            const runRes = await fetch('/run', {
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

            setViewMode('execution')
        } catch (error) {
            console.error(error)
            window.alert('Failed to start pipeline run. Check backend logs for details.')
        } finally {
            setIsRunning(false)
        }
    }

    return (
        <header className="h-14 border-b bg-background flex items-center justify-between px-6 shrink-0 z-50">
            <div className="flex items-center gap-8">
                <div className="flex items-center gap-2">
                    <Settings2 className="w-5 h-5" />
                    <span className="font-semibold tracking-tight">Attractor React</span>
                </div>

                <div className="inline-flex h-10 items-center justify-center rounded-md bg-muted p-1 text-muted-foreground w-[200px]">
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
                </div>
            </div>

            <div className="flex items-center gap-4">
                <div className="flex items-center gap-2">
                    <label htmlFor="model" className="text-sm font-medium text-muted-foreground">Model</label>
                    <input
                        type="text"
                        id="model"
                        placeholder="codex default"
                        value={model}
                        onChange={(e) => setModel(e.target.value)}
                        className="flex h-9 w-40 rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                    />
                </div>
                <div className="flex items-center gap-2">
                    <label htmlFor="workingDir" className="text-sm font-medium text-muted-foreground">Path</label>
                    <input
                        type="text"
                        id="workingDir"
                        value={workingDir}
                        onChange={(e) => setWorkingDir(e.target.value)}
                        className="flex h-9 w-48 rounded-md border border-input bg-transparent px-3 py-1 text-xs font-mono shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                    />
                </div>
                <button
                    onClick={runPipeline}
                    disabled={!activeFlow || isRunning}
                    className="inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0 bg-primary text-primary-foreground shadow hover:bg-primary/90 h-9 px-4 py-2"
                >
                    <Play className="w-4 h-4" />
                    {isRunning ? 'Running...' : 'Deploy Flow'}
                </button>
            </div>
        </header>
    )
}
