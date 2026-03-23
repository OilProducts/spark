import { ReactFlowProvider } from '@xyflow/react'

import { useNarrowViewport } from '@/lib/useNarrowViewport'

import { ExecutionCanvas } from './ExecutionCanvas'
import { ExecutionControls } from './ExecutionControls'
import { ExecutionSidebar } from './ExecutionSidebar'
import { Terminal } from './Terminal'
import { CanvasSessionModeProvider } from './canvasSessionContext'

export function ExecutionWorkspace({ isActive }: { isActive: boolean }) {
    const isNarrowViewport = useNarrowViewport()

    return (
        <section
            data-testid="execution-workspace"
            data-session-active={String(isActive)}
            aria-hidden={!isActive}
            className={`absolute inset-0 ${
                isActive ? 'visible pointer-events-auto' : 'invisible pointer-events-none'
            }`}
        >
            <div className={`flex h-full overflow-hidden ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}>
                <ReactFlowProvider>
                    <CanvasSessionModeProvider mode="execution">
                        <ExecutionSidebar />
                        <div className="flex-1 flex flex-col overflow-hidden">
                            <div className="flex-1 w-full h-full bg-background/50">
                                <ExecutionCanvas />
                                <ExecutionControls />
                            </div>
                            <Terminal />
                        </div>
                    </CanvasSessionModeProvider>
                </ReactFlowProvider>
            </div>
        </section>
    )
}
