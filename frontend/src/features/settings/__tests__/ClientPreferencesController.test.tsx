import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ClientPreferencesController } from '../ClientPreferencesController'
import { completePreferenceInteraction, fetchClientPreferences, saveClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { deferred, switchEnvironment } from '@/test/environments'
import userEvent from '@testing-library/user-event'

vi.mock('../services/clientPreferences', () => ({ fetchClientPreferences: vi.fn(), saveClientPreferences: vi.fn(), completePreferenceInteraction: vi.fn() }))
const initial = useStore.getState()
afterEach(() => { cleanup(); useStore.setState(initial, true); vi.resetAllMocks() })
it('applies persisted preferences and saves completed interactions with revisions, retaining failed drafts', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'files', editor_sidebar_width: 400 }, effective: { editor_mode: 'files', editor_sidebar_width: 400, show_advanced_controls: true, expand_child_flows: true, graph_settings_open: true, home_sidebar_primary_split_ratio: 0.6 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockRejectedValue(new Error('Preferences changed elsewhere.'))
    render(<ClientPreferencesController />)
    await waitFor(() => expect(useStore.getState().editorSidebarWidth).toBe(400))
    expect(useStore.getState().preferredHomeSidebarPrimarySplitRatio).toBe(0.6)
    expect(useStore.getState().preferredEditorMode).toBe('files')
    expect(useStore.getState().preferredAdvancedControls).toBe(true)
    expect(useStore.getState().preferredExpandChildFlows).toBe(true)
    expect(useStore.getState().preferredGraphSettingsOpen).toBe(true)
    expect(saveClientPreferences).not.toHaveBeenCalled()
    act(() => window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: { editor_sidebar_width: 450 } })))
    await screen.findByText('Preferences changed elsewhere.')
    expect(saveClientPreferences).toHaveBeenLastCalledWith(view, { ...view.stored, editor_sidebar_width: 450 }, null)
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    expect(fetchClientPreferences).toHaveBeenCalledTimes(1)
    act(() => window.dispatchEvent(new Event('spark:preferences-retry')))
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledTimes(2))
    expect(saveClientPreferences).toHaveBeenLastCalledWith(view, { ...view.stored, editor_sidebar_width: 450 }, null)
    act(() => window.dispatchEvent(new Event('spark:preferences-discard')))
    await waitFor(() => expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument())
    expect(useStore.getState().editorSidebarWidth).toBe(400)
})

it('ignores a refresh that started before a completed save and keeps the new revision', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'files', editor_sidebar_width: 400 }, effective: { editor_mode: 'files', editor_sidebar_width: 400 } }
    const saved: ClientPreferencesView = { ...view, revision: 'second',
        stored: { ...view.stored, editor_sidebar_width: 450 }, effective: { ...view.effective, editor_sidebar_width: 450 } }
    vi.mocked(fetchClientPreferences).mockResolvedValueOnce(view)
    let finishRefresh!: (value: ClientPreferencesView) => void
    vi.mocked(fetchClientPreferences).mockImplementationOnce(() => new Promise((resolve) => { finishRefresh = resolve }))
    vi.mocked(saveClientPreferences).mockResolvedValue(saved)
    render(<ClientPreferencesController />)
    await waitFor(() => expect(useStore.getState().editorSidebarWidth).toBe(400))
    act(() => window.dispatchEvent(new Event('focus')))
    act(() => {
        useStore.setState({ editorSidebarWidth: 450 })
        window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: { editor_sidebar_width: 450 } }))
    })
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledTimes(1))
    await act(async () => { finishRefresh(view) })
    expect(useStore.getState().editorSidebarWidth).toBe(450)
    act(() => window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: { expand_child_flows: true } })))
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledTimes(2))
    expect(saveClientPreferences).toHaveBeenLastCalledWith(saved, { ...saved.stored, expand_child_flows: true }, null)
})

it('keeps Show thinking on by default, follows the saved setting and persists the toggle', async () => {
    const actual = await vi.importActual<typeof import('../services/clientPreferences')>('../services/clientPreferences')
    vi.mocked(completePreferenceInteraction).mockImplementation(actual.completePreferenceInteraction)
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: null, editor_sidebar_width: null }, effective: { editor_mode: 'structured', editor_sidebar_width: 288, show_thinking: true } }
    const off = { ...view, revision: 'second', stored: { ...view.stored, show_thinking: false }, effective: { ...view.effective, show_thinking: false } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockResolvedValue(off)
    expect(useStore.getState().preferredShowThinking).toBe(true)
    render(<ClientPreferencesController />)
    await waitFor(() => expect(fetchClientPreferences).toHaveBeenCalled())
    await waitFor(() => expect(useStore.getState().clientPreferencesLoaded).toBe(true))
    expect(useStore.getState().preferredShowThinking).toBe(true)
    act(() => useStore.getState().setPreferredShowThinking(false))
    expect(useStore.getState().preferredShowThinking).toBe(false)
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, show_thinking: false }, null))
    // A change saved from the settings editor reaches every transcript through the same state.
    vi.mocked(fetchClientPreferences).mockResolvedValue({ ...off, revision: 'third', effective: { ...off.effective, show_thinking: true } })
    await act(async () => { window.dispatchEvent(new Event('spark:settings-live-event')) })
    await waitFor(() => expect(useStore.getState().preferredShowThinking).toBe(true))
})

it('lets a load failure notice be dismissed, and shows a different problem again', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first', stored: {}, effective: {} }
    vi.mocked(fetchClientPreferences).mockRejectedValueOnce(new Error('down')).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockRejectedValue(new Error('Preferences changed elsewhere.'))
    render(<ClientPreferencesController />)
    const notice = await screen.findByTestId('client-preferences-error')
    act(() => within(notice).getByRole('button', { name: 'Dismiss' }).click())
    expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument()
    act(() => window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: { editor_sidebar_width: 450 } })))
    expect(await screen.findByText('Preferences changed elsewhere.')).toBeVisible()
    act(() => window.dispatchEvent(new Event('spark:preferences-discard')))
    await waitFor(() => expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument())
})

it('keeps an automatic change refused after a switch away for its own workspace, to show and try again there, leaving this machine\'s as it was', async () => {
    const at = (revision: string, graph_height: number): ClientPreferencesView => ({ client_id: 'browser-one', revision,
        stored: { editor_mode: null, editor_sidebar_width: null, run_presentation: { sort: 'newest', graph_height } },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288, run_presentation: { sort: 'newest', graph_height } } })
    const remote = at('remote-1', 400)
    vi.mocked(fetchClientPreferences).mockImplementation(async (environmentId) => (environmentId === null ? at('local-1', 500) : remote))
    const refused = deferred<ClientPreferencesView>()
    vi.mocked(saveClientPreferences).mockReturnValueOnce(refused.promise)
    const changed = { run_presentation: { sort: 'newest' as const, graph_height: 700 } }
    const graphHeight = () => useStore.getState().clientRunPresentation.graph_height

    await switchEnvironment('env-r')
    const remoteShell = render(<ClientPreferencesController />)
    await waitFor(() => expect(graphHeight()).toBe(400))
    act(() => {
        useStore.setState({ clientRunPresentation: changed.run_presentation })
        window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: changed }))
    })
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledWith(remote, { ...remote.stored, ...changed }, 'env-r'))

    // Refused once this machine shows: nothing of it here.
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    remoteShell.unmount()
    const localShell = render(<ClientPreferencesController />)
    await waitFor(() => expect(graphHeight()).toBe(500))
    await act(async () => refused.reject(new Error('The server is unavailable.')))
    expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument()
    expect(graphHeight()).toBe(500)
    expect(saveClientPreferences).toHaveBeenCalledOnce()
    localShell.unmount()

    // Back there, the change shows, unsaved, with why, and Try again saves it there.
    await switchEnvironment('env-r')
    resetStoreForEnvironmentSwitch(false)
    const back = render(<ClientPreferencesController />)
    const notice = await screen.findByTestId('client-preferences-error')
    expect(notice).toHaveTextContent('Couldn’t save your preferences')
    expect(notice).toHaveTextContent('The server is unavailable.')
    await waitFor(() => expect(graphHeight()).toBe(700))
    vi.mocked(saveClientPreferences).mockResolvedValueOnce(at('remote-2', 700))
    await userEvent.setup().click(within(notice).getByRole('button', { name: 'Try again' }))
    await waitFor(() => expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument())
    expect(saveClientPreferences).toHaveBeenLastCalledWith(remote, { ...remote.stored, ...changed }, 'env-r')
    expect(vi.mocked(saveClientPreferences).mock.calls.map(([, , environmentId]) => environmentId)).toEqual(['env-r', 'env-r'])

    // Saved, it is not offered again.
    back.unmount()
    render(<ClientPreferencesController />)
    await waitFor(() => expect(fetchClientPreferences).toHaveBeenCalledTimes(4))
    await act(async () => {})
    expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument()
})
