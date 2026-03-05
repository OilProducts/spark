import { ProjectsPanel } from '@/components/ProjectsPanel'
import { useStore } from '@/store'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const DEFAULT_WORKING_DIRECTORY = './test-app'

const resetProjectScopeState = () => {
  useStore.setState((state) => ({
    ...state,
    viewMode: 'projects',
    activeProjectPath: null,
    activeFlow: null,
    selectedRunId: null,
    workingDir: DEFAULT_WORKING_DIRECTORY,
    projectRegistry: {},
    projectScopedWorkspaces: {},
    projectRegistrationError: null,
    recentProjectPaths: [],
  }))
}

describe('ProjectsPanel', () => {
  beforeEach(() => {
    resetProjectScopeState()
    vi.stubGlobal(
      'fetch',
      vi.fn(async () =>
        new Response(JSON.stringify({ branch: 'main' }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      ),
    )
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('renders quick-switch controls and project event log', () => {
    render(<ProjectsPanel />)

    expect(screen.getByTestId('quick-switch-new-button')).toBeVisible()
    expect(screen.getByTestId('project-directory-picker-input')).toBeInTheDocument()
    expect(screen.getByTestId('quick-switch-controls')).toBeVisible()
    expect(screen.getByTestId('project-event-log-surface')).toBeVisible()
  })

  it('shows an error when picker selection cannot resolve an absolute project path', async () => {
    render(<ProjectsPanel />)
    const pickerInput = screen.getByTestId('project-directory-picker-input') as HTMLInputElement
    const selectedFile = new File(['console.log("hello")'], 'main.ts', { type: 'text/plain' })
    Object.defineProperty(selectedFile, 'webkitRelativePath', {
      configurable: true,
      value: 'quick-switch-project/src/main.ts',
    })
    fireEvent.change(pickerInput, {
      target: {
        files: [selectedFile],
      },
    })
    expect(screen.getByTestId('project-registration-error')).toHaveTextContent(
      'Unable to resolve an absolute project path from the selected directory.',
    )
  })

  it('registers a selected directory from Quick Switch new-button picker', async () => {
    const user = userEvent.setup()
    render(<ProjectsPanel />)

    const pickerInput = screen.getByTestId('project-directory-picker-input') as HTMLInputElement
    const pickerClickSpy = vi.spyOn(pickerInput, 'click')
    await user.click(screen.getByTestId('quick-switch-new-button'))
    expect(pickerClickSpy).toHaveBeenCalled()

    const selectedFile = new File(['console.log("hello")'], 'main.ts', { type: 'text/plain' })
    Object.defineProperty(selectedFile, 'path', {
      configurable: true,
      value: '/tmp/quick-switch-project/src/main.ts',
    })
    Object.defineProperty(selectedFile, 'webkitRelativePath', {
      configurable: true,
      value: 'quick-switch-project/src/main.ts',
    })

    fireEvent.change(pickerInput, {
      target: {
        files: [selectedFile],
      },
    })

    await waitFor(() => {
      expect(useStore.getState().projectRegistry['/tmp/quick-switch-project']).toBeDefined()
    })
    expect(screen.getByTestId('recent-projects-list')).toHaveTextContent('/tmp/quick-switch-project')
    expect(useStore.getState().activeProjectPath).toBe('/tmp/quick-switch-project')
  })
})
