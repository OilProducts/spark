import type { ReactNode } from "react"

interface HomeProjectSidebarProps {
    children: ReactNode
    className?: string
}

export function HomeProjectSidebar({ children, className = "" }: HomeProjectSidebarProps) {
    return (
        <aside
            data-testid="home-project-sidebar"
            className={`flex min-h-0 flex-col ${className}`.trim()}
        >
            {children}
        </aside>
    )
}
