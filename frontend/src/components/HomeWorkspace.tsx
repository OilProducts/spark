import type { ReactNode } from "react"

interface HomeWorkspaceProps {
    children: ReactNode
    className?: string
}

export function HomeWorkspace({ children, className = "" }: HomeWorkspaceProps) {
    return (
        <div data-testid="home-workspace" className={`flex min-h-0 flex-col ${className}`.trim()}>
            {children}
        </div>
    )
}
