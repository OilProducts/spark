import { ProjectConversationHistory } from "@/components/projects/ProjectConversationHistory"
import { ProjectConversationSurface } from "@/components/projects/ProjectConversationSurface"
import { ProjectsSidebar } from "@/components/projects/ProjectsSidebar"
import { useProjectsHomeController } from "@/components/projects/hooks/useProjectsHomeController"

export function ProjectsPanel() {
    const { historyProps, isNarrowViewport, sidebarProps, surfaceProps } = useProjectsHomeController()

    return (
        <section
            data-testid="projects-panel"
            data-home-panel="true"
            data-responsive-layout={isNarrowViewport ? "stacked" : "split"}
            className={`flex-1 ${isNarrowViewport ? "overflow-auto p-3" : "flex min-h-0 flex-col overflow-hidden p-6"}`}
        >
            <div className={`w-full ${isNarrowViewport ? "space-y-6" : "flex min-h-0 flex-1 flex-col gap-6"}`}>
                <div
                    data-testid="home-main-layout"
                    className={`grid gap-4 ${isNarrowViewport ? "grid-cols-1" : "min-h-0 flex-1 grid-cols-[minmax(18rem,22rem)_minmax(0,1fr)]"}`}
                >
                    <ProjectsSidebar {...sidebarProps} />
                    <ProjectConversationSurface
                        {...surfaceProps}
                        historyContent={<ProjectConversationHistory {...historyProps} />}
                    />
                </div>
            </div>
        </section>
    )
}
