import { AppSessionControllers } from "@/app/AppSessionControllers"
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
  const isExecutionMode = viewMode === 'execution'
  const isRunsMode = viewMode === 'runs'
  const isTriggersMode = viewMode === 'triggers'
  const isSettingsMode = viewMode === 'settings'

  return (
    <DialogProvider>
      <AppSessionControllers />
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
          <div
            data-testid="home-workspace-primary"
            data-home-active={String(isHomeMode)}
            className={`absolute inset-0 ${
              isHomeMode ? 'block pointer-events-auto' : 'hidden pointer-events-none'
            }`}
          >
            <ProjectsPanel />
          </div>
          <div
            data-testid="execution-workspace-primary"
            data-execution-active={String(isExecutionMode)}
            className={`absolute inset-0 ${
              isExecutionMode ? 'block pointer-events-auto' : 'hidden pointer-events-none'
            }`}
          >
            <ExecutionWorkspace />
          </div>
          <div
            data-testid="runs-workspace-primary"
            data-runs-active={String(isRunsMode)}
            className={`absolute inset-0 ${
              isRunsMode ? 'block pointer-events-auto' : 'hidden pointer-events-none'
            }`}
          >
            <RunsPanel />
          </div>
          <div
            data-testid="triggers-workspace-primary"
            data-triggers-active={String(isTriggersMode)}
            className={`absolute inset-0 ${
              isTriggersMode ? 'block pointer-events-auto' : 'hidden pointer-events-none'
            }`}
          >
            <TriggersPanel />
          </div>
          {isSettingsMode ? (
            <SettingsPanel />
          ) : null}
        </main>
      </div>
    </DialogProvider>
  )
}

export default App
