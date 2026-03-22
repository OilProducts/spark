import { type KeyboardEvent } from "react"
import { useStore, type ViewMode } from "@/store"
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { Settings2 } from "lucide-react"

const NAV_MODE_ORDER: ViewMode[] = ['home', 'editor', 'execution', 'triggers', 'settings', 'runs']

export function Navbar() {
    const viewMode = useStore((state) => state.viewMode)
    const setViewMode = useStore((state) => state.setViewMode)
    const isNarrowViewport = useNarrowViewport()
    const activeProjectPath = useStore((state) => state.activeProjectPath)

    const resolveNextKeyboardMode = (mode: ViewMode, direction: -1 | 1): ViewMode => {
        const modeCycle: ViewMode[] = NAV_MODE_ORDER
        const currentIndex = modeCycle.indexOf(mode)
        const startIndex = currentIndex >= 0 ? currentIndex : 0
        const nextIndex = (startIndex + direction + modeCycle.length) % modeCycle.length
        return modeCycle[nextIndex]
    }

    const focusModeButton = (mode: ViewMode) => {
        document.querySelector<HTMLButtonElement>(`[data-testid="nav-mode-${mode}"]`)?.focus()
    }

    const onViewModeKeyDown = (event: KeyboardEvent<HTMLButtonElement>, mode: ViewMode) => {
        if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') {
            return
        }
        event.preventDefault()
        const nextMode = resolveNextKeyboardMode(mode, event.key === 'ArrowRight' ? 1 : -1)
        setViewMode(nextMode)
        focusModeButton(nextMode)
    }

    const projectLabel = activeProjectPath || "No active project"

    return (
        <header
            data-testid="top-nav"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
            className={`border-b bg-background shrink-0 z-50 ${isNarrowViewport
                ? 'flex min-h-14 flex-col items-stretch gap-2 px-3 py-2'
                : 'h-14 flex items-center justify-between px-6'
                }`}
        >
            <div className={isNarrowViewport ? 'flex flex-col gap-2' : 'flex items-center gap-8'}>
                <div className="flex items-center gap-2">
                    <Settings2 className="w-5 h-5" />
                    <span className="font-semibold tracking-tight">Spark</span>
                </div>

                <div
                    data-testid="view-mode-tabs"
                    data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
                    className={`inline-flex h-10 items-center justify-center rounded-md bg-muted p-1 text-muted-foreground ${isNarrowViewport ? 'w-full' : 'w-[480px]'}`}
                >
                    <button
                        data-testid="nav-mode-projects"
                        onClick={() => setViewMode('home')}
                        onKeyDown={(event) => onViewModeKeyDown(event, 'home')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${(viewMode === 'home' || viewMode === 'projects') ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        <span data-testid="nav-mode-home">Home</span>
                    </button>
                    <button
                        data-testid="nav-mode-editor"
                        onClick={() => setViewMode('editor')}
                        onKeyDown={(event) => onViewModeKeyDown(event, 'editor')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'editor' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Editor
                    </button>
                    <button
                        data-testid="nav-mode-execution"
                        onClick={() => setViewMode('execution')}
                        onKeyDown={(event) => onViewModeKeyDown(event, 'execution')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'execution' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Execution
                    </button>
                    <button
                        data-testid="nav-mode-triggers"
                        onClick={() => setViewMode('triggers')}
                        onKeyDown={(event) => onViewModeKeyDown(event, 'triggers')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'triggers' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Triggers
                    </button>
                    <button
                        data-testid="nav-mode-settings"
                        onClick={() => setViewMode('settings')}
                        onKeyDown={(event) => onViewModeKeyDown(event, 'settings')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'settings' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Settings
                    </button>
                    <button
                        data-testid="nav-mode-runs"
                        onClick={() => setViewMode('runs')}
                        onKeyDown={(event) => onViewModeKeyDown(event, 'runs')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1.5 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 flex-1 ${viewMode === 'runs' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Runs
                    </button>
                </div>
            </div>
            <div
                data-testid="top-nav-active-project"
                className={`max-w-full truncate rounded border border-border bg-muted/40 px-2 py-1 text-xs text-muted-foreground ${isNarrowViewport ? 'self-start' : 'max-w-80'}`}
            >
                <span className="font-medium text-foreground">Project:</span> {projectLabel}
            </div>
        </header>
    )
}
