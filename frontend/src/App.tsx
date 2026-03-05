import { Navbar } from "./components/Navbar"
import { Sidebar } from "./components/Sidebar"
import { Terminal } from "./components/Terminal"
import { Editor } from "./components/Editor"
import { RunStream } from "./components/RunStream"
import { SettingsPanel } from "./components/SettingsPanel"
import { RunsPanel } from "./components/RunsPanel"
import { ProjectsPanel } from "./components/ProjectsPanel"
import { ReactFlowProvider } from "@xyflow/react"
import { useStore } from "@/store"
import { useNarrowViewport } from "@/lib/useNarrowViewport"

function App() {
  const viewMode = useStore((state) => state.viewMode)
  const isCanvasMode = viewMode === 'editor' || viewMode === 'execution'
  const showSidebar = isCanvasMode
  const isNarrowViewport = useNarrowViewport()

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
            ) : viewMode === 'settings' ? (
              <SettingsPanel />
            ) : viewMode === 'projects' ? (
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
