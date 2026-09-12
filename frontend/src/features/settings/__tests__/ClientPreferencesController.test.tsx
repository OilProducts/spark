import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ClientPreferencesController } from '../ClientPreferencesController'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'
import { useStore } from '@/store'

vi.mock('../services/clientPreferences', () => ({ fetchClientPreferences: vi.fn(), saveClientPreferences: vi.fn(), completePreferenceInteraction: vi.fn() }))
const initial = useStore.getState()
afterEach(() => { cleanup(); useStore.setState(initial, true); vi.resetAllMocks() })
it('applies persisted preferences and saves completed interactions with revisions, retaining failed drafts', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'raw', editor_sidebar_width: 400 }, effective: { editor_mode: 'raw', editor_sidebar_width: 400, show_advanced_controls: true, expand_child_flows: true, graph_settings_open: true, runs_scope: 'all', triggers_scope: 'active', home_sidebar_primary_split_ratio: 0.6 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockRejectedValue(new Error('Preferences changed elsewhere.'))
    render(<ClientPreferencesController />)
    await waitFor(() => expect(useStore.getState().editorSidebarWidth).toBe(400))
    expect(useStore.getState().preferredHomeSidebarPrimarySplitRatio).toBe(0.6)
    expect(useStore.getState().preferredEditorMode).toBe('raw')
    expect(useStore.getState().preferredAdvancedControls).toBe(true)
    expect(useStore.getState().preferredExpandChildFlows).toBe(true)
    expect(useStore.getState().preferredGraphSettingsOpen).toBe(true)
    expect(useStore.getState().runsListSession.scopeMode).toBe('all')
    expect(useStore.getState().triggersSession.scopeFilter).toBe('active')
    expect(saveClientPreferences).not.toHaveBeenCalled()
    act(() => window.dispatchEvent(new CustomEvent('spark:preferences-completed', { detail: { editor_sidebar_width: 450 } })))
    await screen.findByText('Preferences changed elsewhere.')
    expect(saveClientPreferences).toHaveBeenLastCalledWith(view, { ...view.stored, editor_sidebar_width: 450 })
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    expect(fetchClientPreferences).toHaveBeenCalledTimes(1)
    act(() => window.dispatchEvent(new Event('spark:preferences-retry')))
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledTimes(2))
    expect(saveClientPreferences).toHaveBeenLastCalledWith(view, { ...view.stored, editor_sidebar_width: 450 })
})

it('ignores a refresh that started before a completed save and keeps the new revision', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'raw', editor_sidebar_width: 400 }, effective: { editor_mode: 'raw', editor_sidebar_width: 400 } }
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
    expect(saveClientPreferences).toHaveBeenLastCalledWith(saved, { ...saved.stored, expand_child_flows: true })
})
