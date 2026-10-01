import { chooseModel, customModel, openPicker } from '@/components/model-chooser/__tests__/picker'
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { useStore } from '@/store'
import { fetchModelSettings, saveModelSettings, type ModelSettings } from '@/lib/api/settingsApi'
import { fetchClientPreferences } from '../services/clientPreferences'
import { useModelOptions } from '@/components/model-chooser/useModelOptions'
import { SettingsPanel } from '../SettingsPanel'
import { ProjectModelSettingsEditor } from '../ProjectModelSettingsEditor'

// Project model defaults live on the project page; these tests edit them beside the workspace defaults.
const WithProject = () => <><SettingsPanel /><ProjectModelSettingsEditor projectPath="/project" /></>

vi.mock('@/lib/api/settingsApi', () => ({ fetchModelSettings: vi.fn(), saveModelSettings: vi.fn() }))
vi.mock('../services/clientPreferences', async (original) => ({ ...await original<object>(), fetchClientPreferences: vi.fn() }))
vi.mock('@/lib/useLlmProfiles', () => ({ useLlmProfiles: () => [{ id: 'team', label: 'Team models', provider: 'openai_compatible', models: ['team-one'], default_model: 'team-one' }] }))
vi.mock('@/components/model-chooser/useModelOptions', () => ({ useModelOptions: vi.fn() }))
vi.mock('../CodexConnectionSettings', () => ({ CodexConnectionSettings: () => null }))
vi.mock('../ClaudeCodeConnectionSettings', () => ({ ClaudeCodeConnectionSettings: () => null }))
vi.mock('../ProviderSettingsEditor', () => ({ ProviderSettingsEditor: () => null }))
vi.mock('../AgentSettingsEditor', () => ({ AgentSettingsEditor: () => null }))
vi.mock('../ProfileSettingsEditors', () => ({ LlmProfilesEditor: () => null, ExecutionProfilesEditor: () => null }))
vi.mock('../ConnectionSettingsEditor', () => ({ ConnectionSettingsEditor: () => null }))
vi.mock('../RuntimeSettingsEditor', () => ({ RuntimeSettingsEditor: () => null }))

const savedModel = { provider: 'codex', llm_profile: null, model: 'saved-model', reasoning_effort: 'high' }
const card = (name: string) => within(screen.getByRole('heading', { name, exact: true }).closest<HTMLElement>('[data-slot=card]')!)
beforeEach(() => {
    vi.resetAllMocks()
    useStore.setState({ viewMode: 'settings', activeProjectPath: '/project', projectRegistry: { '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: null } } })
    vi.mocked(fetchModelSettings).mockImplementation(async (path, section) => section === 'utility_models'
        ? { scope: 'workspace', source: 'workspace', revision: 'utility-one', stored: null, effective: null }
        : { scope: path ? 'project' : 'workspace', source: 'workspace', revision: 'one', stored: path ? null : savedModel, effective: savedModel })
    vi.mocked(fetchClientPreferences).mockResolvedValue({ client_id: 'browser-test', revision: 'one', stored: { editor_mode: null, editor_sidebar_width: null }, effective: { editor_mode: 'structured', editor_sidebar_width: 288 } })
    vi.mocked(useModelOptions).mockReturnValue({ projectPath: '/project', failed: true, payload: { models: [], providers: { codex: { status: 'unavailable', error: 'offline' } } } })
})
afterEach(() => cleanup())

it('has local default selection, semantic headings, keyboard tabs, and mounted hidden panels', async () => {
    const user = userEvent.setup()
    const view = render(<DialogProvider><SettingsPanel /></DialogProvider>)
    await waitFor(() => expect(card('Model defaults (Workspace)').getByRole('button', { name: /^Model:/ })).toBeEnabled())
    const first = screen.getByRole('tab', { name: 'Models & accounts' })
    expect(first).toHaveAttribute('aria-selected', 'true')
    expect(screen.getAllByRole('tabpanel')).toHaveLength(1)
    expect(screen.getAllByRole('tabpanel', { hidden: true })).toHaveLength(4)
    expect(screen.queryByRole('heading', { name: 'Project model defaults' })).toBeNull()
    first.focus()
    await user.keyboard('{ArrowRight}')
    expect(screen.getByRole('tab', { name: 'Preferences' })).toHaveFocus()
    expect(screen.getByRole('heading', { name: 'Client preferences', level: 3 })).toBeVisible()
    for (const name of ['Editor', 'Layout', 'Runs']) expect(screen.getByRole('heading', { name, level: 4 })).toBeVisible()
    expect(screen.getByLabelText(/^Model:/)).not.toBeVisible()
    await user.keyboard('{End}')
    expect(screen.getByRole('tab', { name: 'System' })).toHaveFocus()
    view.unmount()
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    expect(screen.getByRole('tab', { name: 'Models & accounts' })).toHaveAttribute('aria-selected', 'true')
})

it('retains hidden validation and dirty drafts, protects leaving, and discards per editor', async () => {
    const user = userEvent.setup()
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    await user.click(screen.getByRole('tab', { name: 'Preferences' }))
    const width = await screen.findByLabelText('Editor sidebar width (pixels)')
    await waitFor(() => expect(width).toBeEnabled())
    await user.type(width, '12')
    await user.click(screen.getByRole('tab', { name: 'Execution' }))
    expect(screen.queryByRole('alertdialog')).toBeNull()
    const unload = new Event('beforeunload', { cancelable: true })
    act(() => window.dispatchEvent(unload))
    expect(unload.defaultPrevented).toBe(true)
    await user.click(screen.getByRole('button', { name: 'Open flow editor' }))
    await user.click(screen.getByRole('button', { name: 'Keep editing' }))
    expect(useStore.getState().viewMode).toBe('settings')
    act(() => useStore.getState().setViewMode('runs'))
    await user.click(screen.getByRole('button', { name: 'Keep editing' }))
    expect(useStore.getState().viewMode).toBe('settings')
    await user.click(screen.getByRole('tab', { name: 'Preferences' }))
    expect(width).toHaveValue(12)
    expect(width).toHaveAccessibleDescription(/Choose a whole number from 256 to 560/)
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(width).toHaveValue(null))
    act(() => useStore.getState().setViewMode('runs'))
    expect(useStore.getState().viewMode).toBe('runs')
})

it('shares discovery fallback, profiles, custom values and compatibility with project overrides and labels saved values', async () => {
    const user = userEvent.setup()
    render(<DialogProvider><WithProject /></DialogProvider>)
    const workspace = card('Model defaults (Workspace)')
    const project = card('Project model defaults')
    await waitFor(() => expect(project.getByRole('switch')).toBeEnabled())
    expect(project.getByText(/Saved effective:/)).toHaveTextContent('saved-model')
    expect(project.queryByLabelText('Model')).toBeNull()
    await user.click(project.getByRole('switch'))
    const selectors = screen.getAllByRole('button', { name: /^Model:/ })
    expect(new Set(selectors.map((select) => select.id)).size).toBe(2)
    for (const editor of [workspace, project]) {
        expect(editor.getByRole('button', { name: /^Model:/ })).toHaveTextContent('saved-model')
        await openPicker(user, editor)
        expect(screen.getAllByText(/Model discovery unavailable\./).length).toBeGreaterThan(0)
        await user.keyboard('{Escape}')
        await chooseModel(user, 'openai_compatible / Team models', 'team-one', editor)
        await customModel(user, 'incompatible', editor)
        expect(editor.getByRole('button', { name: /^Model:/ })).toHaveAccessibleDescription('Choose a compatible model for this provider or profile.')
        expect(editor.getByRole('button', { name: /^Save/ })).toBeDisabled()
        await chooseModel(user, 'openai_compatible / Team models', 'team-one', editor)
        expect(editor.getByRole('button', { name: /^Save/ })).toBeEnabled()
    }
    expect(project.getByText(/Saved effective:/)).toHaveTextContent('saved-model')
    await user.click(screen.getByRole('tab', { name: 'System' }))
    await user.click(screen.getByRole('tab', { name: 'Models & accounts' }))
    expect(project.getByRole('button', { name: /^Model:/ })).toHaveTextContent('team-one')
    expect(workspace.getByRole('button', { name: /^Model:/ })).toHaveTextContent('team-one')
})

it('keeps hidden requests mounted and blocks navigation before another dirty editor can offer discard', async () => {
    const user = userEvent.setup()
    vi.mocked(useModelOptions).mockReturnValue({ projectPath: '/project', payload: { models: [{ provider: 'anthropic', id: 'claude-sonnet-4-6', display: 'claude-sonnet-4-6', is_default: false, supported_reasoning_efforts: [] }], providers: { codex: { status: 'unavailable', error: 'offline' }, anthropic: { status: 'available', error: null } } } })
    let reject!: (error: Error) => void
    vi.mocked(saveModelSettings).mockReturnValue(new Promise((_, fail) => { reject = fail }))
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    // Register another dirty editor first to verify pending requests take precedence.
    await user.click(screen.getByRole('tab', { name: 'Preferences' }))
    await waitFor(() => expect(screen.getByLabelText('Editor sidebar width (pixels)')).toBeEnabled())
    await user.type(screen.getByLabelText('Editor sidebar width (pixels)'), '400')
    await user.click(screen.getByRole('tab', { name: 'Models & accounts' }))
    await chooseModel(user, 'anthropic', 'claude-sonnet-4-6', card('Model defaults (Workspace)'))
    await user.click(card('Model defaults (Workspace)').getByRole('button', { name: /^Save/ }))
    await user.click(screen.getByRole('tab', { name: 'Execution' }))
    await user.click(screen.getByRole('button', { name: 'Open flow editor' }))
    act(() => useStore.getState().setViewMode('runs'))
    expect(useStore.getState().viewMode).toBe('settings')
    expect(screen.queryByRole('alertdialog')).toBeNull()
    expect(saveModelSettings).toHaveBeenCalledTimes(1)
    await act(async () => reject(new Error('Save conflict')))
    await user.click(screen.getByRole('tab', { name: 'Models & accounts' }))
    expect(screen.getByText('Save conflict')).toBeVisible()
    expect(card('Model defaults (Workspace)').getByRole('button', { name: /^Model:/ })).toHaveTextContent('claude-sonnet-4-6')
})

it('uses discovered suggestions in both editors and displays inherited saved profile labels and defaults', async () => {
    const user = userEvent.setup()
    const effective = { provider: null, llm_profile: 'team', model: null, reasoning_effort: 'low' }
    vi.mocked(fetchModelSettings).mockImplementation(async (path) => ({ scope: path ? 'project' : 'workspace', source: 'workspace', revision: 'one', stored: path ? null : effective, effective }))
    vi.mocked(useModelOptions).mockReturnValue({ projectPath: '/project', payload: { models: [{ provider: 'codex', id: 'discovered', display: 'Discovered' }], providers: { codex: { status: 'available', error: null } } } })
    render(<DialogProvider><WithProject /></DialogProvider>)
    const project = card('Project model defaults')
    await waitFor(() => expect(project.getByRole('switch')).toBeEnabled())
    expect(project.getByText(/Saved effective:/)).toHaveTextContent('Team models team · Model: team-one · Reasoning effort: low')
    await user.click(project.getByRole('switch'))
    for (const editor of [card('Model defaults (Workspace)'), project]) {
        await chooseModel(user, 'Codex', 'Discovered', editor)
        expect(editor.getByRole('button', { name: /^Model:/ })).toHaveTextContent('Discovered')
    }
    expect(project.getByText(/Saved effective:/)).toHaveTextContent('Team models team')
})

it.each([false, true])('resolves reset from the parent instead of the saved override (project: %s)', async projectScope => {
    const user = userEvent.setup()
    const workspace = { provider: 'anthropic', llm_profile: null, model: 'workspace-parent', reasoning_effort: 'low' }
    const override = { provider: 'anthropic', llm_profile: null, model: 'saved-override', reasoning_effort: 'high' }
    vi.mocked(fetchModelSettings).mockImplementation(async path => ({ scope: path ? 'project' : 'workspace', source: path ? 'project' : 'workspace', revision: 'one',
        stored: path || !projectScope ? override : workspace, effective: path || !projectScope ? override : workspace }))
    vi.mocked(useModelOptions).mockReturnValue({ projectPath: '/project', payload: { models: [{ provider: 'codex', id: 'discovered', display: 'Discovered', is_default: true, default_reasoning_effort: 'medium' }], providers: { codex: { status: 'available', error: null } } } })
    render(<DialogProvider><WithProject /></DialogProvider>)
    const scope = card(projectScope ? 'Project model defaults' : 'Model defaults (Workspace)')
    await waitFor(() => expect(scope.getByRole('button', { name: /^Model:/ })).toHaveTextContent('saved-override · High'))
    await openPicker(user, scope)
    const expected = projectScope ? 'workspace-parent · Low' : 'Discovered · Medium'
    const reset = screen.getByRole('button', { name: /^Use default ·/ })
    expect(reset).toHaveTextContent(`Use default · ${expected}`)
    await user.click(reset)
    expect(scope.getByRole('button', { name: /^Model:/ })).toHaveTextContent(`Default: ${expected}`)
})

it('keeps the utility model off until chosen, offers the chat picker models, and saves or clears it', async () => {
    const user = userEvent.setup()
    vi.mocked(useModelOptions).mockReturnValue({ projectPath: '/project', payload: { models: [
        { provider: 'codex', id: 'discovered', display: 'Discovered' },
        { provider: 'anthropic', id: 'claude-haiku-4-5', display: 'claude-haiku-4-5' },
    ], providers: { codex: { status: 'available', error: null }, anthropic: { status: 'available', error: null } } } })
    const chosen = { provider: 'anthropic', llm_profile: null, model: 'claude-haiku-4-5', reasoning_effort: null }
    let saved = { scope: 'workspace' as const, source: 'workspace' as const, revision: 'utility-one', stored: null as ModelSettings | null, effective: null as ModelSettings | null }
    const models = vi.mocked(fetchModelSettings).getMockImplementation()!
    vi.mocked(fetchModelSettings).mockImplementation(async (path, section) => section === 'utility_models' ? saved : models(path, section))
    vi.mocked(saveModelSettings).mockImplementation(async (revision, value) => {
        saved = { ...saved, revision: `${revision}+`, stored: value, effective: value }
        return saved
    })
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    const utility = card('Utility model')
    await waitFor(() => expect(utility.getByRole('switch')).toBeEnabled())
    expect(utility.getByRole('switch')).not.toBeChecked()
    expect(utility.queryByRole('button', { name: /^Model:/ })).toBeNull()

    await user.click(utility.getByRole('switch'))
    const options = async (scope: ReturnType<typeof card>) => {
        await openPicker(user, scope)
        const names = screen.getAllByRole('option').map((option) => option.textContent)
        await user.keyboard('{Escape}')
        return names
    }
    expect(await options(utility)).toEqual(await options(card('Model defaults (Workspace)')))
    await chooseModel(user, 'anthropic', 'claude-haiku-4-5', utility)
    await user.click(utility.getByRole('button', { name: 'Save utility model' }))
    expect(saveModelSettings).toHaveBeenCalledWith('utility-one', expect.objectContaining(chosen), undefined, 'utility_models')

    await user.click(utility.getByRole('switch'))
    await user.click(utility.getByRole('button', { name: 'Save utility model' }))
    expect(saveModelSettings).toHaveBeenLastCalledWith('utility-one+', null, undefined, 'utility_models')
    await waitFor(() => expect(utility.getByRole('switch')).not.toBeChecked())
})

it.each([
    ['Codex, when both are available', { codex: 'available', 'claude-code': 'available' }, { provider: 'codex', model: 'gpt-5.6-luna' }],
    ['Claude Code, when only it is available', { codex: 'unavailable', 'claude-code': 'available' }, { provider: 'claude-code', model: 'claude-sonnet-5-5' }],
    ['Codex, when neither is available', { codex: 'unavailable', 'claude-code': 'unavailable' }, { provider: 'codex', model: null }],
] as const)('turning the utility model on defaults to %s', async (_label, statuses, expected) => {
    const user = userEvent.setup()
    vi.mocked(useModelOptions).mockReturnValue({ projectPath: '/project', payload: { models: [], providers: Object.fromEntries(
        Object.entries(statuses).map(([provider, status]) => [provider, { status, error: null }]),
    ) } })
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    const utility = card('Utility model')
    await waitFor(() => expect(utility.getByRole('switch')).toBeEnabled())
    await user.click(utility.getByRole('switch'))
    await user.click(utility.getByRole('button', { name: 'Save utility model' }))
    expect(saveModelSettings).toHaveBeenLastCalledWith('utility-one', expect.objectContaining(expected), undefined, 'utility_models')
})
