import { ActivityBar } from "@/app/ActivityBar"
import { AppNotices, AppSessionControllers } from "@/app/AppSessionControllers"
import { EnvironmentBootstrap, EnvironmentSwitcherBar, RemoteEnvironmentGate } from "@/app/EnvironmentSwitcher"
import { EditorWorkspace } from "@/features/editor"
import { ProjectsPanel } from "@/features/projects"
import { RunStream, RunsPanel } from "@/features/runs"
import { TriggersPanel } from "@/features/triggers"
import { isWorkspaceBlocked, useActiveEnvironment } from "@/lib/activeEnvironment"
import { closeWorkspace, useStore } from "@/store"
import { useNarrowViewport } from "@/lib/useNarrowViewport"
import { DialogProvider } from "@/components/app/dialog-controller"
import { Sheet } from "@/components/app/view-layout"
import { MissionsPanel } from "@/features/missions/MissionsPanel"
import { RolesPanel } from "@/features/roles/RolesPanel"
import { RunFlowDialog } from "@/features/launch"
import { lazy, Suspense, useEffect } from "react"

// Screens that mount only while open load on first visit, keeping them out of the startup bundle.
const SettingsPanel = lazy(() => import("@/features/settings/SettingsPanel").then((module) => ({ default: module.SettingsPanel })))
const OverviewPanel = lazy(() => import("@/features/overview/OverviewPanel").then((module) => ({ default: module.OverviewPanel })))

function WorkspaceShell() {
  const viewMode = useStore((state) => state.viewMode)
  const isNarrowViewport = useNarrowViewport()
  const isHomeMode = viewMode === 'home' || viewMode === 'projects'
  const isCanvasMode = viewMode === 'editor'
  const isRunsMode = viewMode === 'runs'
  const isTriggersMode = viewMode === 'triggers'
  const isSettingsMode = viewMode === 'settings'
  // What it started, such as a launch, no longer shows once it closes for a switch to another environment.
  useEffect(() => closeWorkspace, [])

  return (
    <>
      <AppSessionControllers />
      <RunStream />
      <RunFlowDialog />
      <div
        data-testid="app-shell"
        data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
        className={`flex-1 min-h-0 flex antialiased bg-ground text-foreground ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}
      >
        {/* A button, not a #link: the hash is the app's place. */}
        <button
          type="button"
          className="sr-only focus:not-sr-only focus:fixed focus:left-2 focus:top-2 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:shadow-md focus:outline-none focus:ring-2 focus:ring-ring"
          onClick={() => document.getElementById('app-main')?.focus()}
        >
          Skip to content
        </button>
        <ActivityBar />
        <main id="app-main" tabIndex={-1} data-testid="app-main" className="flex-1 min-w-0 min-h-0 relative flex flex-col overflow-hidden outline-none">
          <AppNotices />
          <div className="relative flex min-h-0 flex-1 flex-col">
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
              <div className="absolute inset-0 flex"><Sheet><Suspense><OverviewPanel /></Suspense></Sheet></div>
            ) : null}
            <div hidden={viewMode !== 'missions'} className="absolute inset-0"><MissionsPanel active={viewMode === 'missions'} /></div>
            {viewMode === 'roles' ? <div className="absolute inset-0"><RolesPanel /></div> : null}
            {isSettingsMode ? (
              <Suspense><SettingsPanel /></Suspense>
            ) : null}
          </div>
        </main>
      </div>
    </>
  )
}

function App() {
  const environment = useActiveEnvironment()
  const { activeEnvironmentId } = environment
  // The gate hides the workspace without unmounting it, so its unsaved edits wait for Retry or Return; the gate brings
  // the switcher bar along.
  const blocked = isWorkspaceBlocked(environment)

  return (
    <DialogProvider>
      <EnvironmentBootstrap />
      <div className="h-screen flex flex-col antialiased bg-ground text-foreground">
        {blocked ? null : <EnvironmentSwitcherBar />}
        <div hidden={blocked} className="flex min-h-0 flex-1 flex-col">
          <WorkspaceShell key={activeEnvironmentId ?? 'local'} />
        </div>
        {blocked ? <RemoteEnvironmentGate /> : null}
      </div>
    </DialogProvider>
  )
}

export default App
