import { ReactFlowProvider } from '@xyflow/react'

import { useNarrowViewport } from '@/lib/useNarrowViewport'

import { Editor } from './Editor'
import { Sidebar } from './Sidebar'
import { CanvasSessionModeProvider } from './canvasSessionContext'

export function EditorWorkspace({ isActive }: { isActive: boolean }) {
    const isNarrowViewport = useNarrowViewport()

    return (
        <section
            data-testid="editor-workspace"
            data-session-active={String(isActive)}
            aria-hidden={!isActive}
            className={`absolute inset-0 ${
                isActive ? 'visible pointer-events-auto' : 'invisible pointer-events-none'
            }`}
        >
            <div className={`flex h-full overflow-hidden ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}>
                <ReactFlowProvider>
                    <CanvasSessionModeProvider mode="editor">
                        <Sidebar />
                        <div data-testid="editor-panel" className="flex-1 w-full h-full bg-background/50">
                            <Editor />
                        </div>
                    </CanvasSessionModeProvider>
                </ReactFlowProvider>
            </div>
        </section>
    )
}
