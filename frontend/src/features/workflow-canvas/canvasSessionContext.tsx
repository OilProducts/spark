import { createContext, useContext, type ReactNode } from 'react'

export type CanvasSessionMode = 'editor' | 'runs'

const CanvasRunIdContext = createContext<string | null>(null)

export const useCanvasRunId = () => useContext(CanvasRunIdContext)

const CanvasSessionModeContext = createContext<CanvasSessionMode>('editor')

export function CanvasSessionModeProvider({
    mode,
    runId = null,
    children,
}: {
    mode: CanvasSessionMode
    runId?: string | null
    children: ReactNode
}) {
    return (
        <CanvasSessionModeContext.Provider value={mode}>
            <CanvasRunIdContext.Provider value={runId}>{children}</CanvasRunIdContext.Provider>
        </CanvasSessionModeContext.Provider>
    )
}

export function useCanvasSessionMode(): CanvasSessionMode {
    return useContext(CanvasSessionModeContext)
}
