import App from '@/App'
import { useStore } from '@/store'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const resetAppShellState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    executionFlow: null,
    selectedRunId: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    runtimeStatus: 'idle',
    diagnostics: [],
    nodeDiagnostics: {},
    edgeDiagnostics: {},
    hasValidationErrors: false,
    logs: [],
    humanGate: null,
    nodeStatuses: {},
    selectedNodeId: null,
    selectedEdgeId: null,
    projectRegistry: {},
    projectScopedWorkspaces: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
    graphAttrs: {},
    graphAttrErrors: {},
    saveState: 'idle',
    saveErrorMessage: null,
    saveErrorKind: null,
  }))
}

const resolveRequestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

describe('App shell behavior', () => {
  beforeEach(() => {
    resetAppShellState()
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL) => {
        const url = resolveRequestUrl(input)
        if (url.includes('/attractor/api/flows')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/status')) {
          return new Response(JSON.stringify({ status: 'idle', last_run_id: null }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/workspace/api/projects')) {
          return new Response(JSON.stringify([]), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        if (url.includes('/attractor/runs')) {
          return new Response(JSON.stringify({ runs: [] }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          })
        }
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      }),
    )
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('renders shell regions and switches among projects/settings/runs modes', async () => {
    const user = userEvent.setup()
    render(<App />)

    expect(screen.getByTestId('app-shell')).toBeVisible()
    expect(screen.getByTestId('app-main')).toBeVisible()
    expect(screen.getByTestId('top-nav')).toBeVisible()
    expect(screen.getByTestId('projects-panel')).toBeVisible()
    expect(screen.getByTestId('top-nav-active-project')).toHaveTextContent('No active project')
    expect(screen.queryByTestId('top-nav-active-flow')).not.toBeInTheDocument()
    expect(screen.queryByTestId('top-nav-run-context')).not.toBeInTheDocument()

    await user.click(screen.getByTestId('nav-mode-settings'))
    expect(useStore.getState().viewMode).toBe('settings')
    expect(screen.getByTestId('settings-panel')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-runs'))
    expect(useStore.getState().viewMode).toBe('runs')
    expect(screen.getByTestId('runs-panel')).toBeVisible()

    await user.click(screen.getByTestId('nav-mode-projects'))
    expect(useStore.getState().viewMode).toBe('home')
    expect(screen.getByTestId('projects-panel')).toBeVisible()
  })

  it('prevents editor navigation without an active project and allows it after project selection', async () => {
    const user = userEvent.setup()
    render(<App />)

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect(useStore.getState().viewMode).toBe('home')
    expect(screen.getByTestId('projects-panel')).toBeVisible()

    act(() => {
      useStore.getState().registerProject('/tmp/project-shell')
    })

    expect(screen.getByTestId('top-nav-active-project')).toHaveTextContent('/tmp/project-shell')

    await user.click(screen.getByTestId('nav-mode-editor'))
    expect(useStore.getState().viewMode).toBe('editor')
    expect(screen.getByTestId('canvas-workspace-primary')).toBeVisible()
    expect(screen.getByTestId('inspector-panel')).toBeVisible()
    expect(screen.getByTestId('editor-no-flow-state')).toHaveTextContent('Select a flow to begin authoring.')
  })
})
