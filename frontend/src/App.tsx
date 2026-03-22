import { Navbar } from "./components/Navbar"
import { Sidebar } from "./components/Sidebar"
import { Terminal } from "./components/Terminal"
import { Editor } from "./components/Editor"
import { RunStream } from "./components/RunStream"
import { SettingsPanel } from "./components/SettingsPanel"
import { RunsPanel } from "./components/RunsPanel"
import { ProjectsPanel } from "./components/ProjectsPanel"
import { TriggersPanel } from "./components/TriggersPanel"
import { ReactFlowProvider } from "@xyflow/react"
import { useStore } from "@/store"
import { useNarrowViewport } from "@/lib/useNarrowViewport"
import { fetchProjectRegistryValidated } from "@/lib/workspaceClient"
import { useEffect } from "react"

function App() {
  const viewMode = useStore((state) => state.viewMode)
  const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
  const isHomeMode = viewMode === 'home' || viewMode === 'projects'
  const isCanvasMode = viewMode === 'editor' || viewMode === 'execution'
  const showSidebar = isCanvasMode
  const isNarrowViewport = useNarrowViewport()

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
    <ReactFlowProvider>
      <RunStream />
      <div data-testid="app-shell" className="h-screen flex flex-col antialiased bg-background text-foreground">
        <Navbar />
        <div className={`flex-1 flex overflow-hidden ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}>
          {showSidebar && <Sidebar />}
          <main data-testid="app-main" className="flex-1 relative flex flex-col overflow-hidden bg-muted/10">
            {isCanvasMode ? (
              <div data-testid="canvas-workspace-primary" className="flex-1 flex flex-col overflow-hidden">
                <div data-testid="editor-panel" className="flex-1 w-full h-full bg-background/50">
                  <Editor />
                </div>
                <Terminal />
              </div>
            ) : viewMode === 'triggers' ? (
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
      </div>
    </ReactFlowProvider>
  )
}

export default App
