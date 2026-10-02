import { ActivityBar } from "@/app/ActivityBar"
import { AppSessionControllers } from "@/app/AppSessionControllers"
import { EditorWorkspace } from "@/features/editor"
import { ProjectsPanel } from "@/features/projects"
import { RunStream, RunsPanel } from "@/features/runs"
import { SettingsPanel } from "@/features/settings"
import { TriggersPanel } from "@/features/triggers"
import { useStore } from "@/store"
import { useNarrowViewport } from "@/lib/useNarrowViewport"
import { DialogProvider } from "@/components/app/dialog-controller"
import { MissionsPanel } from "@/features/missions/MissionsPanel"
import { OverviewPanel } from "@/features/overview/OverviewPanel"
function App() {
  const viewMode = useStore((state) => state.viewMode)
  const isNarrowViewport = useNarrowViewport()
  const isHomeMode = viewMode === 'home' || viewMode === 'projects'
  const isCanvasMode = viewMode === 'editor'
  const isRunsMode = viewMode === 'runs'
  const isTriggersMode = viewMode === 'triggers'
  const isSettingsMode = viewMode === 'settings'

  return (
    <DialogProvider>
      <AppSessionControllers />
      <RunStream />
      <div
        data-testid="app-shell"
        data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
        className={`h-screen flex antialiased bg-background text-foreground ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}
      >
        <ActivityBar />
        <main data-testid="app-main" className="flex-1 min-w-0 min-h-0 relative flex flex-col overflow-hidden">
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
          {viewMode === 'overview' ? (
            <div className="absolute inset-0"><OverviewPanel /></div>
          ) : null}
          <div hidden={viewMode !== 'missions'} className="absolute inset-0"><MissionsPanel active={viewMode === 'missions'} /></div>
          {isSettingsMode ? (
            <SettingsPanel />
          ) : null}
        </main>
      </div>
    </DialogProvider>
  )
}

export default App
