import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { flowLoadFailed } from './model/flowLoadFailure'
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { ReactFlowProvider } from '@xyflow/react'
import { ArrowLeft } from 'lucide-react'

import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'

import { Editor } from './Editor'
import { EditorGraphBridgeProvider } from './EditorGraphBridgeContext'
import { Sidebar } from './Sidebar'
import { selectFlowSettingsOpen } from './GraphSettings'
import { FlowsPanelActions, FlowsPanelList, useFlowCatalogPanel } from './FlowsPanel'
import { ViewLayout } from '@/components/app/view-layout'
import { Button } from '@/components/ui/button'
import { CanvasSessionModeProvider } from '@/features/workflow-canvas'

const MIN_EDITOR_SIDEBAR_WIDTH = 256
const MIN_EDITOR_CANVAS_WIDTH = 480
const MAX_EDITOR_SIDEBAR_WIDTH = 560
const EDITOR_SIDEBAR_RESIZE_HANDLE_WIDTH = 12

function clampEditorSidebarWidth(width: number, containerWidth: number) {
    if (containerWidth <= 0) {
        return Math.min(Math.max(width, MIN_EDITOR_SIDEBAR_WIDTH), MAX_EDITOR_SIDEBAR_WIDTH)
    }
    const maxSidebarWidth = Math.min(
        MAX_EDITOR_SIDEBAR_WIDTH,
        Math.max(
            MIN_EDITOR_SIDEBAR_WIDTH,
            containerWidth - MIN_EDITOR_CANVAS_WIDTH - EDITOR_SIDEBAR_RESIZE_HANDLE_WIDTH,
        ),
    )
    return Math.min(Math.max(width, MIN_EDITOR_SIDEBAR_WIDTH), maxSidebarWidth)
}

export function EditorWorkspace({ isActive }: { isActive: boolean }) {
    const isNarrowViewport = useNarrowViewport()
    const editorSidebarWidth = useStore((state) => state.editorSidebarWidth)
    const setEditorSidebarWidth = useStore((state) => state.setEditorSidebarWidth)
    const activeFlow = useStore((state) => state.activeFlow)
    // A flow that failed to load has no inspector to resize.
    const hasActiveFlow = useStore((state) => Boolean(state.activeFlow) && !flowLoadFailed(state.flowLoadFailure, state.activeFlow, state.activeProjectPath))
    // Files mode has no inspector either.
    const isFilesMode = useStore((state) => state.editorMode === 'files')
    // With its Flow settings closed, a flow with nothing selected shows its canvas alone.
    const showInspector = useStore((state) => Boolean(state.selectedNodeId || state.selectedEdgeId) || selectFlowSettingsOpen(state))
    const catalog = useFlowCatalogPanel()
    const [isEditorSidebarResizing, setIsEditorSidebarResizing] = useState(false)
    const workspaceRef = useRef<HTMLDivElement | null>(null)
    const editorSidebarResizeRef = useRef<{ startX: number; startWidth: number } | null>(null)
    const effectiveIsEditorSidebarResizing = isEditorSidebarResizing && !isNarrowViewport
    // A narrowing window takes width from the sidebar, down to its minimum, before the canvas; the saved width returns as it widens.
    const [workspaceWidth, setWorkspaceWidth] = useState(0)
    useEffect(() => {
        const element = workspaceRef.current
        if (!element || typeof ResizeObserver === 'undefined') return
        const observer = new ResizeObserver(([entry]) => setWorkspaceWidth(entry.contentRect.width))
        observer.observe(element)
        return () => observer.disconnect()
    }, [])
    const sidebarWidth = clampEditorSidebarWidth(editorSidebarWidth, workspaceWidth)

    // Stacked, the flow list is above the open flow: go back up to it, onto the open flow's row.
    const showFlowList = () => {
        const panel = workspaceRef.current?.closest('[data-testid="flows-view"]')?.querySelector<HTMLElement>('[data-testid="side-panel"]')
        panel?.scrollIntoView({ block: 'start' })
        panel?.querySelector<HTMLElement>('[aria-current="true"]')?.focus()
    }

    const persistSidebarWidth = () => completePreferenceInteraction({ editor_sidebar_width: useStore.getState().editorSidebarWidth })

    const adjustEditorSidebarWidth = (delta: number) => {
        const containerWidth = workspaceRef.current?.getBoundingClientRect().width || 0
        if (containerWidth <= 0) {
            return
        }
        setEditorSidebarWidth(clampEditorSidebarWidth(sidebarWidth + delta, containerWidth))
        persistSidebarWidth()
    }

    const onEditorSidebarResizePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
        if (isNarrowViewport) {
            return
        }
        editorSidebarResizeRef.current = {
            startX: event.clientX,
            startWidth: sidebarWidth,
        }
        setIsEditorSidebarResizing(true)
        document.body.style.cursor = 'col-resize'
        document.body.style.userSelect = 'none'
        event.preventDefault()
    }

    const onEditorSidebarResizeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.key === 'ArrowLeft') {
            event.preventDefault()
            adjustEditorSidebarWidth(-24)
            return
        }
        if (event.key === 'ArrowRight') {
            event.preventDefault()
            adjustEditorSidebarWidth(24)
            return
        }
        const containerWidth = workspaceRef.current?.getBoundingClientRect().width || 0
        if (containerWidth <= 0) {
            return
        }
        if (event.key === 'Home') {
            event.preventDefault()
            setEditorSidebarWidth(clampEditorSidebarWidth(MIN_EDITOR_SIDEBAR_WIDTH, containerWidth))
            persistSidebarWidth()
            return
        }
        if (event.key === 'End') {
            event.preventDefault()
            setEditorSidebarWidth(clampEditorSidebarWidth(MAX_EDITOR_SIDEBAR_WIDTH, containerWidth))
            persistSidebarWidth()
        }
    }

    useEffect(() => {
        if (isNarrowViewport) {
            editorSidebarResizeRef.current = null
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
            return
        }
        return () => {
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
        }
    }, [isNarrowViewport])

    useEffect(() => {
        if (!effectiveIsEditorSidebarResizing) {
            return
        }

        const stopEditorSidebarResize = () => {
            completePreferenceInteraction({ editor_sidebar_width: useStore.getState().editorSidebarWidth })
            setIsEditorSidebarResizing(false)
            editorSidebarResizeRef.current = null
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
        }

        const handleEditorSidebarPointerMove = (event: PointerEvent) => {
            const resizeState = editorSidebarResizeRef.current
            const containerWidth = workspaceRef.current?.getBoundingClientRect().width || 0
            if (!resizeState || containerWidth <= 0) {
                return
            }
            const nextWidth = resizeState.startWidth + (event.clientX - resizeState.startX)
            setEditorSidebarWidth(clampEditorSidebarWidth(nextWidth, containerWidth))
        }

        window.addEventListener('pointermove', handleEditorSidebarPointerMove)
        window.addEventListener('pointerup', stopEditorSidebarResize)
        window.addEventListener('pointercancel', stopEditorSidebarResize)
        return () => {
            window.removeEventListener('pointermove', handleEditorSidebarPointerMove)
            window.removeEventListener('pointerup', stopEditorSidebarResize)
            window.removeEventListener('pointercancel', stopEditorSidebarResize)
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
        }
    }, [effectiveIsEditorSidebarResizing, setEditorSidebarWidth])

    return (
        <section
            data-testid="editor-workspace"
            data-session-active={String(isActive)}
            aria-hidden={!isActive}
            className={`absolute inset-0 ${
                isActive ? 'block pointer-events-auto' : 'hidden pointer-events-none'
            }`}
        >
            <ViewLayout
                view="flows"
                title="Flows"
                actions={<FlowsPanelActions catalog={catalog} />}
                panel={<FlowsPanelList catalog={catalog} />}
                selected={activeFlow}
            >
            <div
                ref={workspaceRef}
                className={`flex h-full overflow-hidden ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}
            >
                {isNarrowViewport && activeFlow ? (
                    <div className="shrink-0 px-3 pt-2">
                        <Button type="button" variant="link" size="sm" className="h-auto p-0 text-muted-foreground" onClick={showFlowList}>
                            <ArrowLeft aria-hidden="true" />
                            All flows
                        </Button>
                    </div>
                ) : null}
                <ReactFlowProvider>
                    <CanvasSessionModeProvider mode="editor">
                        <EditorGraphBridgeProvider>
                            {isNarrowViewport || !showInspector ? null : <Sidebar desktopWidthPx={sidebarWidth} />}
                            {!isNarrowViewport && hasActiveFlow && !isFilesMode && showInspector ? (
                                <div
                                    data-testid="editor-sidebar-resize-handle"
                                    role="separator"
                                    aria-label="Resize editor sidebar"
                                    aria-orientation="vertical"
                                    tabIndex={0}
                                    onPointerDown={onEditorSidebarResizePointerDown}
                                    onKeyDown={onEditorSidebarResizeKeyDown}
                                    className={`group flex w-3 shrink-0 cursor-col-resize items-center justify-center focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${
                                        effectiveIsEditorSidebarResizing ? 'bg-muted' : 'hover:bg-muted/60'
                                    }`}
                                >
                                    <span className="h-16 w-1 rounded-full bg-border transition-colors group-hover:bg-muted-foreground/70" />
                                </div>
                            ) : null}
                            <div data-testid="editor-panel" className="flex-1 min-h-0 min-w-0 w-full h-full">
                                <Editor isActive={isActive} flowList={{ failed: catalog.loadFailed, retry: () => void catalog.refreshFlows() }} />
                            </div>
                            {/* Stacked, the canvas comes first, right under the flow list, and the inspector follows. */}
                            {isNarrowViewport && showInspector ? <Sidebar desktopWidthPx={sidebarWidth} /> : null}
                        </EditorGraphBridgeProvider>
                    </CanvasSessionModeProvider>
                </ReactFlowProvider>
            </div>
            </ViewLayout>
        </section>
    )
}
