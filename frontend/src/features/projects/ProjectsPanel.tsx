import { useState } from "react"

import { ProjectConversationHistory } from "./components/ProjectConversationHistory"
import { ProjectConversationSurface } from "./components/ProjectConversationSurface"
import { ProjectsSidebar } from "./components/ProjectsSidebar"
import { useProjectsHomeController } from "./hooks/useProjectsHomeController"
import { useProjectRegistryBootstrap } from "./hooks/useProjectRegistryBootstrap"
import { useStore } from "@/store"

export function ProjectsPanel() {
    const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
    const shouldBootstrapRegistry = useStore((state) => Object.keys(state.projectRegistry).length === 0)
    const [registryBootstrapError, setRegistryBootstrapError] = useState<string | null>(null)
    const { historyProps, isNarrowViewport, sidebarProps, surfaceProps } = useProjectsHomeController()

    useProjectRegistryBootstrap({
        hydrateProjectRegistry,
        enabled: shouldBootstrapRegistry,
        onError: setRegistryBootstrapError,
    })

    return (
        <section
            data-testid="projects-panel"
            data-home-panel="true"
            data-responsive-layout={isNarrowViewport ? "stacked" : "split"}
            className={`h-full flex-1 ${isNarrowViewport ? "overflow-auto p-3" : "flex min-h-0 flex-col overflow-hidden p-6"}`}
        >
            <div className={`w-full ${isNarrowViewport ? "space-y-6" : "flex min-h-0 flex-1 flex-col gap-6"}`}>
                <div
                    data-testid="home-main-layout"
                    className={`grid gap-4 ${isNarrowViewport ? "grid-cols-1" : "min-h-0 flex-1 grid-cols-[minmax(18rem,22rem)_minmax(0,1fr)]"}`}
                >
                    <ProjectsSidebar {...sidebarProps} />
                    <ProjectConversationSurface
                        {...surfaceProps}
                        panelError={registryBootstrapError || surfaceProps.panelError}
                        historyContent={<ProjectConversationHistory {...historyProps} />}
                    />
                </div>
            </div>
        </section>
    )
}
