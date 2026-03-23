import { createContext, useContext, type ReactNode } from 'react'

export type CanvasSessionMode = 'editor' | 'execution'

const CanvasSessionModeContext = createContext<CanvasSessionMode>('editor')

export function CanvasSessionModeProvider({
    mode,
    children,
}: {
    mode: CanvasSessionMode
    children: ReactNode
}) {
    return (
        <CanvasSessionModeContext.Provider value={mode}>
            {children}
        </CanvasSessionModeContext.Provider>
    )
}

export function useCanvasSessionMode(): CanvasSessionMode {
    return useContext(CanvasSessionModeContext)
}
