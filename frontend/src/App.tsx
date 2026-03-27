import { Navbar } from "@/app/Navbar"
import { EditorWorkspace } from "@/features/editor"
import { ExecutionWorkspace } from "@/features/execution"
import { ProjectsPanel } from "@/features/projects"
import { RunStream, RunsPanel } from "@/features/runs"
import { SettingsPanel } from "@/features/settings"
import { TriggersPanel } from "@/features/triggers"
import { useStore } from "@/store"
import { DialogProvider } from "@/ui"

function App() {
  const viewMode = useStore((state) => state.viewMode)
  const isHomeMode = viewMode === 'home' || viewMode === 'projects'
  const isCanvasMode = viewMode === 'editor'

  return (
    <DialogProvider>
      <RunStream />
      <div data-testid="app-shell" className="h-screen flex flex-col antialiased bg-background text-foreground">
        <Navbar />
        <main data-testid="app-main" className="flex-1 relative flex flex-col overflow-hidden bg-muted/10">
          <div
            data-testid="canvas-workspace-primary"
            data-canvas-active={String(isCanvasMode)}
            className={`absolute inset-0 ${
              isCanvasMode ? 'block pointer-events-auto' : 'hidden pointer-events-none'
            }`}
          >
            <EditorWorkspace isActive={viewMode === 'editor'} />
          </div>
          {viewMode === 'triggers' ? (
            <TriggersPanel />
          ) : viewMode === 'settings' ? (
            <SettingsPanel />
          ) : isHomeMode ? (
            <ProjectsPanel />
          ) : viewMode === 'execution' ? (
            <ExecutionWorkspace />
          ) : viewMode === 'runs' ? (
            <RunsPanel />
          ) : null}
        </main>
      </div>
    </DialogProvider>
  )
}

export default App
