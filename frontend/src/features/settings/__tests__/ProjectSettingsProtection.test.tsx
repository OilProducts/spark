import { customModel } from '@/components/model-chooser/__tests__/picker'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { ProjectExecutionSettings } from '@/features/projects/components/ProjectExecutionSettings'
import { ProjectPage } from '@/features/projects/components/ProjectPage'
import { useStore } from '@/store'
import { fetchModelSettings, fetchProjectExecutionSettings } from '@/lib/api/settingsApi'
import { deleteProjectValidated, fetchWorkspaceSettingsValidated, updateProjectStateValidated, type WorkspaceSettingsResponse } from '@/lib/workspaceClient'
import { ProjectModelSettingsEditor } from '../ProjectModelSettingsEditor'

vi.mock('@/lib/api/settingsApi', () => ({ fetchModelSettings: vi.fn(), fetchProjectExecutionSettings: vi.fn(), saveModelSettings: vi.fn() }))
vi.mock('@/lib/workspaceClient', async (original) => ({ ...await original<object>(), deleteProjectValidated: vi.fn(), fetchWorkspaceSettingsValidated: vi.fn(), updateProjectStateValidated: vi.fn() }))
vi.mock('@/components/model-chooser/useModelOptions', () => ({ useModelOptions: () => ({ projectPath: '/project-one', payload: { providers: { codex: { status: 'available', error: null } }, models: [] } }) }))
vi.mock('@/lib/useLlmProfiles', () => ({ useLlmProfiles: () => [] }))

beforeEach(() => {
    vi.clearAllMocks()
    useStore.setState({ activeProjectPath: '/project-one', projectPagePath: '/project-one', viewMode: 'home', projectRegistry: {
        '/project-one': { directoryPath: '/project-one', isFavorite: false, lastAccessedAt: null },
        '/project-two': { directoryPath: '/project-two', isFavorite: false, lastAccessedAt: null },
    } })
    vi.mocked(fetchModelSettings).mockResolvedValue({ scope: 'project', source: 'workspace', revision: 'models-1', stored: null,
        effective: { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null } })
    vi.mocked(fetchProjectExecutionSettings).mockResolvedValue({ revision: 'execution-1', stored: null })
    vi.mocked(fetchWorkspaceSettingsValidated).mockResolvedValue({ execution_placement: {
        config: { loaded: true }, validation_errors: [], profiles: [{ id: 'native', label: 'Native', enabled: true, mode: 'native' }],
    } } as WorkspaceSettingsResponse)
})

function Models() {
    const project = useStore((state) => state.projectPagePath)
    return project ? <ProjectModelSettingsEditor key={project} projectPath={project} /> : <p>No project</p>
}

it.each(['switch', 'switch_and_leave', 'clear', 'remove', 'hydrate', 'rename'] as const)('retains a model draft when %s is cancelled and applies the confirmed transition', async (transition) => {
    const user = userEvent.setup()
    render(<DialogProvider><Models /></DialogProvider>)
    await waitFor(() => expect(screen.getByRole('switch')).toBeEnabled())
    await user.click(screen.getByRole('switch'))
    await customModel(user, 'unsaved-model')
    const navigate = () => {
        const state = useStore.getState()
        if (transition === 'switch') state.openProjectPage('/project-two')
        if (transition === 'switch_and_leave') { state.openProjectPage('/project-two'); state.setViewMode('runs') }
        if (transition === 'clear') state.openProjectPage(null)
        if (transition === 'remove') state.removeProject('/project-one', '/project-two')
        if (transition === 'hydrate') state.hydrateProjectRegistry([])
        if (transition === 'rename') state.updateProjectPath('/project-one', '/project-renamed')
    }
    await act(async () => navigate())
    await user.click(await screen.findByRole('button', { name: 'Keep editing' }))
    expect(useStore.getState().projectPagePath).toBe('/project-one')
    expect(screen.getByRole('button', { name: /^Model:/ })).toHaveTextContent('unsaved-model')
    await act(async () => navigate())
    await user.click(await screen.findByRole('button', { name: 'Discard and leave' }))
    await waitFor(() => expect(useStore.getState().projectPagePath).not.toBe('/project-one'))
    expect(screen.queryByDisplayValue('unsaved-model')).not.toBeInTheDocument()
    if (transition === 'switch_and_leave') expect(useStore.getState().viewMode).toBe('runs')
})

function Execution() {
    const page = useStore((state) => state.projectPagePath)
    return page ? <ProjectExecutionSettings projectPath={page} /> : <p>Left the project page</p>
}

it('confirms leaving the project page and retains the execution draft on cancellation', async () => {
    useStore.setState({ projectPagePath: '/project-one', viewMode: 'home' })
    const user = userEvent.setup()
    render(<DialogProvider><Execution /></DialogProvider>)
    await waitFor(() => expect(screen.getByTestId('project-default-execution-profile')).toBeEnabled())
    await user.click(screen.getByTestId('project-default-execution-profile'))
    await user.click(screen.getByRole('option', { name: 'Native (native)' }))
    act(() => useStore.getState().openProjectPage(null))
    await user.click(await screen.findByRole('button', { name: 'Keep editing' }))
    expect(screen.getByTestId('project-default-execution-profile')).toHaveTextContent('Native')
    // Other views keep the project page mounted, so switching views keeps the draft without asking.
    act(() => useStore.getState().setViewMode('runs'))
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(screen.getByTestId('project-default-execution-profile')).toHaveTextContent('Native')
    act(() => useStore.getState().openProjectPage(null))
    await user.click(await screen.findByRole('button', { name: 'Discard and leave' }))
    await waitFor(() => expect(screen.queryByTestId('project-settings-dialog')).not.toBeInTheDocument())
    expect(updateProjectStateValidated).not.toHaveBeenCalled()
})

it('refetches clean execution settings, retains dirty drafts on live changes and conflicts, and reloads explicitly', async () => {
    useStore.setState({ projectPagePath: '/project-one' })
    const user = userEvent.setup()
    render(<DialogProvider><Execution /></DialogProvider>)
    const select = screen.getByTestId('project-default-execution-profile')
    await waitFor(() => expect(select).toBeEnabled())
    vi.mocked(fetchProjectExecutionSettings).mockResolvedValue({ revision: 'execution-2', stored: 'native' })
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await waitFor(() => expect(select).toHaveTextContent('Native'))
    await user.click(select)
    await user.click(screen.getByRole('option', { name: 'Use workspace default' }))
    vi.mocked(fetchProjectExecutionSettings).mockResolvedValue({ revision: 'execution-3', stored: 'native' })
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    expect(await screen.findByRole('status')).toHaveTextContent('Your draft is retained')
    expect(select).toHaveTextContent('Use workspace default')
    vi.mocked(updateProjectStateValidated).mockRejectedValue(new Error('Revision conflict'))
    await user.click(screen.getByTestId('project-settings-save-button'))
    expect(await screen.findByTestId('project-settings-save-error')).toHaveTextContent('Revision conflict')
    expect(updateProjectStateValidated).toHaveBeenCalledWith({ project_path: '/project-one', expected_revision: 'execution-2', execution_profile_id: null })
    expect(select).toHaveTextContent('Use workspace default')
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(select).toHaveTextContent('Native'))
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
})

it('blocks navigation and duplicate saves while an execution update is pending', async () => {
    useStore.setState({ projectPagePath: '/project-one' })
    const user = userEvent.setup()
    render(<DialogProvider><Execution /></DialogProvider>)
    await waitFor(() => expect(screen.getByTestId('project-default-execution-profile')).toBeEnabled())
    await user.click(screen.getByTestId('project-default-execution-profile'))
    await user.click(screen.getByRole('option', { name: 'Native (native)' }))
    let reject!: (error: Error) => void
    vi.mocked(updateProjectStateValidated).mockReturnValue(new Promise((_, fail) => { reject = fail }))
    await user.click(screen.getByTestId('project-settings-save-button'))
    expect(screen.getByTestId('project-settings-save-button')).toBeDisabled()
    expect(screen.getByRole('button', { name: /^Discard/ })).toBeDisabled()
    act(() => useStore.getState().openProjectPage(null))
    expect(useStore.getState().projectPagePath).toBe('/project-one')
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(screen.getByTestId('project-settings-dialog')).toBeVisible()
    expect(updateProjectStateValidated).toHaveBeenCalledTimes(1)
    await act(async () => reject(new Error('Save failed')))
    expect(screen.getByTestId('project-default-execution-profile')).toHaveTextContent('Native')
    expect(screen.getByTestId('project-settings-save-error')).toHaveTextContent('Save failed')
})

function Page() {
    const page = useStore((state) => state.projectPagePath)
    return page ? (
        <ProjectPage projectPath={page} chats={[]} formatConversationAgeShort={() => ''} pendingDeleteConversationId={null}
            onCreateConversationThread={() => {}} onSelectConversationThread={() => {}} onDeleteConversationThread={() => {}} />
    ) : <p>Left the project page</p>
}

const removeFromPage = async (user: ReturnType<typeof userEvent.setup>) => {
    await user.click(screen.getByTestId('project-page-remove'))
    await user.click(await screen.findByRole('button', { name: 'Remove project' }))
}

it('asks about unsaved settings before removing a project, and only deletes once leaving is allowed', async () => {
    vi.mocked(deleteProjectValidated).mockResolvedValue(undefined as never)
    const user = userEvent.setup()
    render(<DialogProvider><Page /></DialogProvider>)
    await waitFor(() => expect(screen.getByRole('switch')).toBeEnabled())
    await user.click(screen.getByRole('switch'))
    await customModel(user, 'unsaved-model')

    await removeFromPage(user)
    await user.click(await screen.findByRole('button', { name: 'Keep editing' }))
    expect(deleteProjectValidated).not.toHaveBeenCalled()
    expect(useStore.getState().projectRegistry['/project-one']).toBeDefined()
    expect(screen.getByRole('button', { name: /^Model:/ })).toHaveTextContent('unsaved-model')

    await removeFromPage(user)
    await user.click(await screen.findByRole('button', { name: 'Discard and leave' }))
    await screen.findByText('Left the project page')
    expect(deleteProjectValidated).toHaveBeenCalledWith('/project-one')
    expect(useStore.getState().projectRegistry['/project-one']).toBeUndefined()
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
})

it('does not remove a project while its settings are saving', async () => {
    const user = userEvent.setup()
    render(<DialogProvider><Page /></DialogProvider>)
    await waitFor(() => expect(screen.getByTestId('project-default-execution-profile')).toBeEnabled())
    await user.click(screen.getByTestId('project-default-execution-profile'))
    await user.click(screen.getByRole('option', { name: 'Native (native)' }))
    vi.mocked(updateProjectStateValidated).mockReturnValue(new Promise(() => {}))
    await user.click(screen.getByTestId('project-settings-save-button'))

    await removeFromPage(user)
    expect(deleteProjectValidated).not.toHaveBeenCalled()
    expect(useStore.getState().projectPagePath).toBe('/project-one')
    expect(useStore.getState().projectRegistry['/project-one']).toBeDefined()
})
