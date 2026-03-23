import { Navbar } from "./components/Navbar"
import { RunStream } from "./components/RunStream"
import { SettingsPanel } from "./components/SettingsPanel"
import { RunsPanel } from "./components/RunsPanel"
import { ProjectsPanel } from "./components/ProjectsPanel"
import { TriggersPanel } from "./components/TriggersPanel"
import { EditorWorkspace } from "./components/EditorWorkspace"
import { ExecutionWorkspace } from "./components/ExecutionWorkspace"
import { useStore } from "@/store"
import { fetchProjectRegistryValidated } from "@/lib/workspaceClient"
import { useEffect } from "react"

function App() {
  const viewMode = useStore((state) => state.viewMode)
  const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
  const isHomeMode = viewMode === 'home' || viewMode === 'projects'
  const isCanvasMode = viewMode === 'editor' || viewMode === 'execution'

  useEffect(() => {
    let canceled = false

    const loadProjectRegistry = async () => {
      try {
        const projects = await fetchProjectRegistryValidated()
        if (!canceled) {
          hydrateProjectRegistry(
            projects.map((project) => ({
              directoryPath: project.project_path,
              isFavorite: project.is_favorite,
              lastAccessedAt: project.last_accessed_at ?? null,
              activeConversationId: project.active_conversation_id ?? null,
            })),
          )
        }
      } catch (error) {
        console.error(error)
      }
    }

    void loadProjectRegistry()
    return () => {
      canceled = true
    }
  }, [hydrateProjectRegistry])

  return (
    <>
      <RunStream />
      <div data-testid="app-shell" className="h-screen flex flex-col antialiased bg-background text-foreground">
        <Navbar />
        <main data-testid="app-main" className="flex-1 relative flex flex-col overflow-hidden bg-muted/10">
          <div
            data-testid="canvas-workspace-primary"
            data-canvas-active={String(isCanvasMode)}
            className={`absolute inset-0 ${
              isCanvasMode ? 'visible pointer-events-auto' : 'invisible pointer-events-none'
            }`}
          >
            <EditorWorkspace isActive={viewMode === 'editor'} />
            <ExecutionWorkspace isActive={viewMode === 'execution'} />
          </div>
          {viewMode === 'triggers' ? (
            <TriggersPanel />
          ) : viewMode === 'settings' ? (
            <SettingsPanel />
          ) : isHomeMode ? (
            <ProjectsPanel />
          ) : viewMode === 'runs' ? (
            <RunsPanel />
          ) : null}
        </main>
      </div>
    </>
  )
}

export default App
