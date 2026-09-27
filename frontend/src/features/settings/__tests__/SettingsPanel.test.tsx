import { chooseModel, customModel, openPicker } from '@/components/model-chooser/__tests__/picker'
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { useStore } from '@/store'
import { fetchModelSettings, saveModelSettings } from '@/lib/api/settingsApi'
import { fetchClientPreferences } from '../services/clientPreferences'
import { useModelOptions } from '@/components/model-chooser/useModelOptions'
import { SettingsPanel } from '../SettingsPanel'

vi.mock('@/lib/api/settingsApi', () => ({ fetchModelSettings: vi.fn(), saveModelSettings: vi.fn() }))
vi.mock('../services/clientPreferences', async (original) => ({ ...await original<object>(), fetchClientPreferences: vi.fn() }))
vi.mock('@/lib/useLlmProfiles', () => ({ useLlmProfiles: () => [{ id: 'team', label: 'Team models', provider: 'openai_compatible', models: ['team-one'], default_model: 'team-one' }] }))
vi.mock('@/components/model-chooser/useModelOptions', () => ({ useModelOptions: vi.fn() }))
vi.mock('../CodexConnectionSettings', () => ({ CodexConnectionSettings: () => null }))
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
    vi.mocked(fetchModelSettings).mockImplementation(async (path) => ({ scope: path ? 'project' : 'workspace', source: 'workspace', revision: 'one', stored: path ? null : savedModel, effective: savedModel }))
    vi.mocked(fetchClientPreferences).mockResolvedValue({ client_id: 'browser-test', revision: 'one', stored: { editor_mode: null, editor_sidebar_width: null }, effective: { editor_mode: 'structured', editor_sidebar_width: 288 } })
    vi.mocked(useModelOptions).mockReturnValue({ projectPath: '/project', failed: true })
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
    expect(screen.getByRole('heading', { name: 'Project model defaults', level: 3 })).toBeVisible()
    first.focus()
    await user.keyboard('{ArrowRight}')
    expect(screen.getByRole('tab', { name: 'Preferences' })).toHaveFocus()
    expect(screen.getByRole('heading', { name: 'Client preferences', level: 3 })).toBeVisible()
    for (const name of ['Editor', 'Layout', 'Runs & triggers']) expect(screen.getByRole('heading', { name, level: 4 })).toBeVisible()
    expect(screen.getByLabelText(/^Model:/)).not.toBeVisible()
    await user.keyboard('{End}')
    expect(screen.getByRole('tab', { name: 'System' })).toHaveFocus()
    view.unmount()
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    expect(screen.getByRole('tab', { name: 'Models & accounts' })).toHaveAttribute('aria-selected', 'true')
})

it('retains hidden validation and dirty drafts, protects leaving and project changes, and discards per editor', async () => {
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
    act(() => useStore.getState().setActiveProjectPath(null))
    await user.click(screen.getByRole('button', { name: 'Keep editing' }))
    expect(useStore.getState().activeProjectPath).toBe('/project')
    await user.click(screen.getByRole('tab', { name: 'Preferences' }))
    expect(width).toHaveValue(12)
    expect(width).toHaveAccessibleDescription(/Choose a whole number from 256 to 560/)
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(width).toHaveValue(null))
    act(() => useStore.getState().setActiveProjectPath(null))
    expect(useStore.getState().activeProjectPath).toBeNull()
})

it('shares discovery fallback, profiles, custom values and compatibility with project overrides and labels saved values', async () => {
    const user = userEvent.setup()
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
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
        expect(screen.getAllByText('Model discovery unavailable. Using suggestions.').length).toBeGreaterThan(0)
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
    act(() => useStore.getState().setActiveProjectPath(null))
    expect(useStore.getState().viewMode).toBe('settings')
    expect(useStore.getState().activeProjectPath).toBe('/project')
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
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
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
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    const scope = card(projectScope ? 'Project model defaults' : 'Model defaults (Workspace)')
    await waitFor(() => expect(scope.getByRole('button', { name: /^Model:/ })).toHaveTextContent('saved-override · High'))
    await openPicker(user, scope)
    const expected = projectScope ? 'workspace-parent · Low' : 'Discovered · Medium'
    const reset = screen.getByRole('button', { name: /^Use default ·/ })
    expect(reset).toHaveTextContent(`Use default · ${expected}`)
    await user.click(reset)
    expect(scope.getByRole('button', { name: /^Model:/ })).toHaveTextContent(`Default: ${expected}`)
})
