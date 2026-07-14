import { type KeyboardEvent, useEffect, useRef, useState } from "react"
import { useStore, type ViewMode } from "@/store"
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { Bell, Plus, Settings2, SlidersHorizontal, Trash2, X } from "lucide-react"
import { fetchPendingAttention, type AttentionItem } from "@/lib/api/attentionApi"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { ProjectBrowserDialog } from './ProjectBrowserDialog'
import { ProjectSettingsDialog } from './ProjectSettingsDialog'
import { useProjectSwitcherControls } from './useProjectSwitcherControls'
import { formatProjectListLabel } from '@/features/projects/model/projectsHomeState'

const NAV_MODE_ORDER: ViewMode[] = ['home', 'editor', 'triggers', 'settings', 'runs']
const NAV_MODE_BUTTON_CLASS = 'flex-1 rounded-sm px-3 py-1.5 text-sm'

const NAV_MODE_ITEMS: Array<{
    buttonTestId: string
    label: string
    labelTestId?: string
    mode: ViewMode
}> = [
    {
        buttonTestId: 'nav-mode-projects',
        label: 'Home',
        labelTestId: 'nav-mode-home',
        mode: 'home',
    },
    {
        buttonTestId: 'nav-mode-editor',
        label: 'Editor',
        mode: 'editor',
    },
    {
        buttonTestId: 'nav-mode-triggers',
        label: 'Triggers',
        mode: 'triggers',
    },
    {
        buttonTestId: 'nav-mode-settings',
        label: 'Settings',
        mode: 'settings',
    },
    {
        buttonTestId: 'nav-mode-runs',
        label: 'Runs',
        mode: 'runs',
    },
]

const ATTENTION_POLL_MS = 30_000

const ATTENTION_KIND_LABELS: Record<AttentionItem['kind'], string> = {
    run_gate: 'Run waiting at gate',
    flow_run_request: 'Flow run request',
    proposed_plan: 'Plan pending review',
}

function AttentionBell() {
    const setViewMode = useStore((state) => state.setViewMode)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const updateProjectSessionState = useStore((state) => state.updateProjectSessionState)
    const [items, setItems] = useState<AttentionItem[]>([])
    const [open, setOpen] = useState(false)
    const containerRef = useRef<HTMLDivElement | null>(null)

    useEffect(() => {
        let disposed = false
        const refresh = () => {
            fetchPendingAttention()
                .then((next) => {
                    if (!disposed) {
                        setItems(next)
                    }
                })
                .catch(() => {
                    // Transient poll failures keep the last known items.
                })
        }
        refresh()
        const interval = window.setInterval(refresh, ATTENTION_POLL_MS)
        const onFocus = () => refresh()
        window.addEventListener('focus', onFocus)
        return () => {
            disposed = true
            window.clearInterval(interval)
            window.removeEventListener('focus', onFocus)
        }
    }, [])

    useEffect(() => {
        if (!open) {
            return
        }
        const onPointerDown = (event: PointerEvent) => {
            if (!containerRef.current?.contains(event.target as Node)) {
                setOpen(false)
            }
        }
        window.addEventListener('pointerdown', onPointerDown)
        return () => window.removeEventListener('pointerdown', onPointerDown)
    }, [open])

    const openItem = (item: AttentionItem) => {
        setOpen(false)
        if (item.project_path) {
            setActiveProjectPath(item.project_path)
        }
        if (item.kind === 'run_gate' && item.run_id) {
            setSelectedRunId(item.run_id)
            setViewMode('runs')
            return
        }
        if (item.conversation_id && item.project_path) {
            updateProjectSessionState(item.project_path, { conversationId: item.conversation_id })
        }
        setViewMode('projects')
    }

    return (
        <div ref={containerRef} className="relative">
            <Button
                type="button"
                data-testid="attention-bell"
                variant="ghost"
                size="xs"
                aria-label={items.length > 0 ? `${items.length} items waiting on you` : 'Nothing waiting on you'}
                onClick={() => setOpen((previous) => !previous)}
                className="relative"
            >
                <Bell className="h-4 w-4" />
                {items.length > 0 && (
                    <span
                        data-testid="attention-bell-count"
                        className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-amber-500 px-1 text-[10px] font-semibold text-white"
                    >
                        {items.length}
                    </span>
                )}
            </Button>
            {open && (
                <div
                    data-testid="attention-bell-list"
                    className="absolute right-0 top-full z-50 mt-1 w-96 rounded-md border bg-background p-1 shadow-md"
                >
                    {items.length === 0 && (
                        <div className="px-2 py-1.5 text-xs text-muted-foreground">
                            Nothing is waiting on you.
                        </div>
                    )}
                    {items.map((item) => (
                        <button
                            key={`${item.kind}:${item.id}`}
                            type="button"
                            data-testid="attention-bell-item"
                            onClick={() => openItem(item)}
                            className="flex w-full flex-col items-start gap-0.5 rounded-sm px-2 py-1.5 text-left hover:bg-muted"
                        >
                            <span className="text-[10px] font-semibold uppercase tracking-wide text-amber-600">
                                {ATTENTION_KIND_LABELS[item.kind]}
                            </span>
                            <span className="w-full truncate text-xs">
                                {item.title || item.run_id || item.id}
                            </span>
                            <span className="w-full truncate text-[10px] text-muted-foreground">
                                {formatProjectListLabel(item.project_path)}
                                {item.conversation_handle ? ` · ${item.conversation_handle}` : ''}
                            </span>
                        </button>
                    ))}
                </div>
            )}
        </div>
    )
}

export function Navbar() {
    const viewMode = useStore((state) => state.viewMode)
    const setViewMode = useStore((state) => state.setViewMode)
    const isNarrowViewport = useNarrowViewport()
    const [projectSettingsOpen, setProjectSettingsOpen] = useState(false)
    const {
        activeProjectPath,
        isProjectBrowserLoading,
        isProjectBrowserOpen,
        orderedProjects,
        projectBrowserErrorMessage,
        projectBrowserState,
        projectErrorMessage,
        onActivateProject,
        onBrowseProjectDirectory,
        onClearActiveProject,
        onDeleteActiveProject,
        onOpenProjectDirectoryChooser,
        onSelectProjectBrowserDirectory,
        onSetProjectBrowserOpen,
    } = useProjectSwitcherControls()
    const projectSwitcherValue = activeProjectPath ?? '__no-active-project__'

    const resolveNextKeyboardMode = (mode: ViewMode, direction: -1 | 1): ViewMode => {
        const modeCycle: ViewMode[] = NAV_MODE_ORDER
        const currentIndex = modeCycle.indexOf(mode)
        const startIndex = currentIndex >= 0 ? currentIndex : 0
        const nextIndex = (startIndex + direction + modeCycle.length) % modeCycle.length
        return modeCycle[nextIndex]
    }

    const resolveModeButtonTestId = (mode: ViewMode) => (
        NAV_MODE_ITEMS.find((item) => item.mode === mode)?.buttonTestId || `nav-mode-${mode}`
    )

    const focusModeButton = (mode: ViewMode) => {
        document.querySelector<HTMLButtonElement>(`[data-testid="${resolveModeButtonTestId(mode)}"]`)?.focus()
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

    const hasRegisteredProjects = orderedProjects.length > 0
    const closedProjectLabel = activeProjectPath
        ? formatProjectListLabel(activeProjectPath)
        : hasRegisteredProjects
            ? 'Choose project'
            : 'No projects'

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
                    {NAV_MODE_ITEMS.map((item) => {
                        const isActive = item.mode === 'home'
                            ? (viewMode === 'home' || viewMode === 'projects')
                            : viewMode === item.mode
                        return (
                            <Button
                                key={item.mode}
                                type="button"
                                data-testid={item.buttonTestId}
                                aria-current={isActive ? 'page' : undefined}
                                onClick={() => setViewMode(item.mode)}
                                onKeyDown={(event) => onViewModeKeyDown(event, item.mode)}
                                variant={isActive ? 'secondary' : 'ghost'}
                                className={NAV_MODE_BUTTON_CLASS}
                            >
                                {item.labelTestId ? (
                                    <span data-testid={item.labelTestId}>{item.label}</span>
                                ) : item.label}
                            </Button>
                        )
                    })}
                </div>
            </div>
            <AttentionBell />
            <div
                data-testid="top-nav-active-project"
                data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
                className={`max-w-full ${isNarrowViewport ? 'w-full space-y-2' : 'w-[360px] space-y-1'}`}
            >
                <div className={`flex items-center gap-2 ${isNarrowViewport ? 'flex-wrap' : ''}`}>
                    <Select
                        value={projectSwitcherValue}
                        onValueChange={(value) => {
                            if (value === '__no-active-project__') {
                                return
                            }
                            void onActivateProject(value)
                        }}
                    >
                        <SelectTrigger
                            data-testid="top-nav-project-switcher"
                            size="sm"
                            title={activeProjectPath || 'No active project'}
                            className={`${isNarrowViewport ? 'min-w-0 flex-1' : 'min-w-0 flex-1'} bg-muted/40`}
                        >
                            <SelectValue placeholder={hasRegisteredProjects ? 'Choose project' : 'No projects'}>
                                {closedProjectLabel}
                            </SelectValue>
                        </SelectTrigger>
                        <SelectContent align="end">
                            <SelectItem value="__no-active-project__" title="No active project">
                                <div className="flex min-w-0 flex-col">
                                    <span className="truncate font-medium">
                                        {hasRegisteredProjects ? 'Choose project' : 'No projects'}
                                    </span>
                                    <span className="truncate text-[10px] text-muted-foreground">
                                        {activeProjectPath || 'No active project selected'}
                                    </span>
                                </div>
                            </SelectItem>
                            {orderedProjects.map((project) => (
                                <SelectItem
                                    key={project.directoryPath}
                                    value={project.directoryPath}
                                    title={project.directoryPath}
                                >
                                    <div className="flex min-w-0 flex-col">
                                        <span className="truncate font-medium">
                                            {formatProjectListLabel(project.directoryPath)}
                                        </span>
                                        <span className="truncate text-[10px] text-muted-foreground">
                                            {project.directoryPath}
                                        </span>
                                    </div>
                                </SelectItem>
                            ))}
                        </SelectContent>
                    </Select>
                    <Button
                        data-testid="top-nav-project-add-button"
                        type="button"
                        onClick={() => {
                            void onOpenProjectDirectoryChooser()
                        }}
                        variant="outline"
                        size="xs"
                    >
                        <Plus className="h-3.5 w-3.5" />
                        Add
                    </Button>
                    <Button
                        data-testid="top-nav-project-clear-button"
                        type="button"
                        onClick={onClearActiveProject}
                        variant="outline"
                        size="xs"
                        disabled={!activeProjectPath}
                    >
                        <X className="h-3.5 w-3.5" />
                        Clear
                    </Button>
                    <Button
                        data-testid="top-nav-project-settings-button"
                        type="button"
                        onClick={() => setProjectSettingsOpen(true)}
                        variant="outline"
                        size="xs"
                        disabled={!activeProjectPath}
                        aria-label="Project settings"
                        title="Project settings"
                    >
                        <SlidersHorizontal className="h-3.5 w-3.5" />
                    </Button>
                    {activeProjectPath ? (
                        <Button
                            data-testid="top-nav-project-remove-button"
                            type="button"
                            onClick={() => {
                                void onDeleteActiveProject()
                            }}
                            variant="outline"
                            size="xs"
                            className="border-destructive/40 text-destructive hover:bg-destructive/10"
                        >
                            <Trash2 className="h-3.5 w-3.5" />
                            Remove
                        </Button>
                    ) : null}
                </div>
                {projectErrorMessage ? (
                    <p data-testid="top-nav-project-error" className="text-xs text-destructive">
                        {projectErrorMessage}
                    </p>
                ) : null}
            </div>
            <ProjectBrowserDialog
                open={isProjectBrowserOpen}
                currentPath={projectBrowserState?.current_path ?? null}
                parentPath={projectBrowserState?.parent_path ?? null}
                roots={projectBrowserState?.roots ?? []}
                entries={projectBrowserState?.entries ?? []}
                errorMessage={projectBrowserErrorMessage}
                isLoading={isProjectBrowserLoading}
                onBrowse={onBrowseProjectDirectory}
                onOpenChange={onSetProjectBrowserOpen}
                onSelectCurrentFolder={onSelectProjectBrowserDirectory}
            />
            <ProjectSettingsDialog
                open={projectSettingsOpen}
                projectPath={activeProjectPath}
                onOpenChange={setProjectSettingsOpen}
            />
        </header>
    )
}
