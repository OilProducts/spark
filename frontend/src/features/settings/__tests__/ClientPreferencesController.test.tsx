import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ClientPreferencesController } from '../ClientPreferencesController'
import { completePreferenceInteraction, fetchClientPreferences, saveClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { applyTheme, cachedAppearance, type Appearance } from '@/lib/theme'
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
    await waitFor(() => expect(fetchClientPreferences).toHaveBeenCalledTimes(3))
    await act(async () => {})
    expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument()
})

it('keeps a newer setting made elsewhere when an automatic change refused after a switch away is tried again there', async () => {
    const at = (revision: string, sort: 'newest' | 'oldest', graph_height: number): ClientPreferencesView => ({ client_id: 'browser-one', revision,
        stored: { editor_mode: null, editor_sidebar_width: null, run_presentation: { sort, graph_height } },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288, run_presentation: { sort, graph_height } } })
    // The remote server, which refuses a save based on a revision it no longer has.
    let remote = at('remote-1', 'newest', 400)
    vi.mocked(fetchClientPreferences).mockImplementation(async (environmentId) => (environmentId === null ? at('local-1', 'newest', 500) : remote))
    const refused = deferred<ClientPreferencesView>()
    vi.mocked(saveClientPreferences).mockReturnValueOnce(refused.promise).mockImplementation(async (base, preferences) => {
        if (base.revision !== remote.revision) throw new Error('Preferences changed elsewhere.')
        return remote = { ...base, revision: `${base.revision}+`, stored: preferences, effective: { ...base.effective, ...preferences } as ClientPreferencesView['effective'] }
    })
    const changed = { run_presentation: { sort: 'newest' as const, graph_height: 700 } }
    const presentation = () => useStore.getState().clientRunPresentation

    await switchEnvironment('env-r')
    const remoteShell = render(<ClientPreferencesController />)
    await waitFor(() => expect(presentation().graph_height).toBe(400))
    act(() => window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: changed })))
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledOnce())
    await switchEnvironment(null)
    resetStoreForEnvironmentSwitch(true)
    remoteShell.unmount()
    await act(async () => refused.reject(new Error('The server is unavailable.')))
    // Meanwhile another client sorts runs there oldest first.
    remote = at('remote-2', 'oldest', 400)

    await switchEnvironment('env-r')
    resetStoreForEnvironmentSwitch(false)
    render(<ClientPreferencesController />)
    const user = userEvent.setup()
    await user.click(within(await screen.findByTestId('client-preferences-error')).getByRole('button', { name: 'Try again' }))
    expect(await screen.findByText('Preferences changed elsewhere.')).toBeVisible()
    expect(remote).toEqual(at('remote-2', 'oldest', 400))
    await user.click(within(screen.getByTestId('client-preferences-error')).getByRole('button', { name: 'Discard changes' }))
    await waitFor(() => expect(presentation()).toEqual({ sort: 'oldest', graph_height: 400 }))
    expect(screen.queryByTestId('client-preferences-error')).not.toBeInTheDocument()
})

const appearanceView = (appearance: Appearance): ClientPreferencesView => ({ client_id: 'browser-one', revision: appearance,
    stored: { editor_mode: null, editor_sidebar_width: null, appearance }, effective: { editor_mode: 'structured', editor_sidebar_width: 288, appearance } })
// A page opened anew in `environmentId`, this machine's server storing Dark: its modules load fresh, and it
// shows the browser's appearance without recording it, as main.tsx does at startup.
const openPage = async (environmentId: string | null) => {
    vi.resetModules()
    const services = await import('../services/clientPreferences')
    vi.mocked(services.fetchClientPreferences).mockImplementation(async (id) => appearanceView(id === null ? 'dark' : 'light'))
    const environments = await import('@/test/environments')
    await environments.switchEnvironment(environmentId)
    const theme = await import('@/lib/theme')
    theme.applyTheme(theme.cachedAppearance(), false)
    const { ClientPreferencesController: Controller } = await import('../ClientPreferencesController')
    // Shows the workspace of `id`, as a switch remounts it, once its preferences have loaded.
    const show = async (id: string | null) => {
        cleanup()
        await environments.switchEnvironment(id)
        const loads = vi.mocked(services.fetchClientPreferences).mock.calls.length
        render(<Controller />)
        await waitFor(() => expect(services.fetchClientPreferences).toHaveBeenCalledTimes(loads + 1))
        await act(async () => {})
    }
    return show
}

it('restores this machine\'s saved appearance on the page\'s first load, to a new browser, and on no later or remote load', async () => {
    // A new browser, which has chosen no appearance, takes the one this machine saved.
    localStorage.removeItem('spark.appearance')
    let show = await openPage(null)
    await show(null)
    expect(cachedAppearance()).toBe('dark')
    expect(document.documentElement).toHaveClass('dark')
    // Back on this machine later, after System is picked here, the browser keeps it.
    applyTheme('system')
    await show('env-r')
    await show(null)
    expect(cachedAppearance()).toBe('system')

    // Nor does a page opened here in a browser that chose its appearance.
    applyTheme('light')
    show = await openPage(null)
    await show(null)
    expect(cachedAppearance()).toBe('light')

    // A page opened on a remote environment in a new browser takes no appearance from it, nor later from this machine.
    localStorage.removeItem('spark.appearance')
    show = await openPage('env-r')
    await show('env-r')
    await show(null)
    expect(cachedAppearance()).toBe('system')
    expect(document.documentElement).not.toHaveClass('dark')
})

it('keeps System saved from remote Settings across a reload into this machine, whose server stores Dark', async () => {
    applyTheme('light')
    let show = await openPage('env-r')
    await show('env-r')
    // Settings there saves System, and applies it to this browser once saved.
    applyTheme('system')

    show = await openPage(null)
    await show(null)
    expect(localStorage.getItem('spark.appearance')).toBe('system')
    expect(document.documentElement).not.toHaveClass('dark')
})
