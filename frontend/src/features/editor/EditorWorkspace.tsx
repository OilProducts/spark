import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import { ReactFlowProvider } from '@xyflow/react'

import { useStore } from '@/store'
import { useNarrowViewport } from '@/lib/useNarrowViewport'

import { Editor } from './Editor'
import { Sidebar } from './Sidebar'
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
    const [isEditorSidebarResizing, setIsEditorSidebarResizing] = useState(false)
    const workspaceRef = useRef<HTMLDivElement | null>(null)
    const editorSidebarResizeRef = useRef<{ startX: number; startWidth: number } | null>(null)

    const adjustEditorSidebarWidth = (delta: number) => {
        const containerWidth = workspaceRef.current?.getBoundingClientRect().width || 0
        if (containerWidth <= 0) {
            return
        }
        setEditorSidebarWidth(clampEditorSidebarWidth(editorSidebarWidth + delta, containerWidth))
    }

    const onEditorSidebarResizePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
        if (isNarrowViewport) {
            return
        }
        editorSidebarResizeRef.current = {
            startX: event.clientX,
            startWidth: editorSidebarWidth,
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
            return
        }
        if (event.key === 'End') {
            event.preventDefault()
            setEditorSidebarWidth(clampEditorSidebarWidth(MAX_EDITOR_SIDEBAR_WIDTH, containerWidth))
        }
    }

    useEffect(() => {
        if (isNarrowViewport) {
            setIsEditorSidebarResizing(false)
            editorSidebarResizeRef.current = null
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
            return
        }

        const syncEditorSidebarWidth = () => {
            const containerWidth = workspaceRef.current?.getBoundingClientRect().width || 0
            if (containerWidth <= 0) {
                return
            }
            const nextWidth = clampEditorSidebarWidth(editorSidebarWidth, containerWidth)
            if (nextWidth !== editorSidebarWidth) {
                setEditorSidebarWidth(nextWidth)
            }
        }

        syncEditorSidebarWidth()
        window.addEventListener('resize', syncEditorSidebarWidth)
        return () => {
            window.removeEventListener('resize', syncEditorSidebarWidth)
            document.body.style.cursor = ''
            document.body.style.userSelect = ''
        }
    }, [editorSidebarWidth, isNarrowViewport, setEditorSidebarWidth])

    useEffect(() => {
        if (!isEditorSidebarResizing) {
            return
        }

        const stopEditorSidebarResize = () => {
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
    }, [isEditorSidebarResizing, setEditorSidebarWidth])

    return (
        <section
            data-testid="editor-workspace"
            data-session-active={String(isActive)}
            aria-hidden={!isActive}
            className={`absolute inset-0 ${
                isActive ? 'block pointer-events-auto' : 'hidden pointer-events-none'
            }`}
        >
            <div
                ref={workspaceRef}
                className={`flex h-full overflow-hidden ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}
            >
                <ReactFlowProvider>
                    <CanvasSessionModeProvider mode="editor">
                        <Sidebar desktopWidthPx={editorSidebarWidth} />
                        {!isNarrowViewport ? (
                            <div
                                data-testid="editor-sidebar-resize-handle"
                                role="separator"
                                aria-label="Resize editor sidebar"
                                aria-orientation="vertical"
                                tabIndex={0}
                                onPointerDown={onEditorSidebarResizePointerDown}
                                onKeyDown={onEditorSidebarResizeKeyDown}
                                className={`group flex w-3 shrink-0 cursor-col-resize items-center justify-center focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${
                                    isEditorSidebarResizing ? 'bg-muted' : 'hover:bg-muted/60'
                                }`}
                            >
                                <span className="h-16 w-1 rounded-full bg-border transition-colors group-hover:bg-muted-foreground/70" />
                            </div>
                        ) : null}
                        <div data-testid="editor-panel" className="flex-1 min-w-0 w-full h-full bg-background/50">
                            <Editor />
                        </div>
                    </CanvasSessionModeProvider>
                </ReactFlowProvider>
            </div>
        </section>
    )
}
