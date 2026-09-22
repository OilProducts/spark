import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { SettingsPanel } from '../SettingsPanel'

vi.mock('@/lib/useLlmProfiles', () => ({ useLlmProfiles: () => [] }))
vi.mock('../hooks/useModelDiscovery', () => ({ useModelDiscovery: () => null }))
vi.mock('../hooks/useWorkspaceSettings', () => ({ useWorkspaceSettings: () => ({ workspaceSettings: null, settingsError: null }) }))
vi.mock('../hooks/useModelSettingsEditor', () => ({ useModelSettingsEditor: () => ({ saved: null, draft: null, pending: false, dirty: false, error: '', message: '', setDraft: vi.fn(), save: vi.fn(), discard: vi.fn() }) }))
vi.mock('../RuntimeSettingsEditor', () => ({ RuntimeSettingsEditor: () => null }))

afterEach(() => { cleanup(); delete window.__TAURI__ })

it('saves Desktop changes explicitly with confirmation and a revision, retaining conflicts until discard', async () => {
    const user = userEvent.setup()
    const original = {
        revision: 'core-1', remote_access_enabled: false, bind_host: '127.0.0.1',
        server_url: 'http://127.0.0.1:45678/', requires_restart: false,
        remote_access_warning: 'Other devices can reach this server.',
    }
    const invoke = vi.fn().mockImplementation((command: string) => command === 'desktop_server_settings'
        ? Promise.resolve(original) : Promise.reject(new Error('Settings changed. Reload before saving.')))
    window.__TAURI__ = { core: { invoke } }
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    await user.click(screen.getByRole('tab', { name: 'System' }))
    const toggle = await screen.findByRole('switch', { name: 'Remote desktop server access' })
    await user.click(toggle)
    expect(invoke.mock.calls.filter(([command]) => command === 'set_desktop_remote_access_enabled')).toHaveLength(0)
    expect(toggle).toBeChecked()
    const save = screen.getByRole('button', { name: 'Save' })
    await user.click(save)
    expect(save).toBeDisabled()
    await user.click(screen.getByRole('button', { name: 'Cancel', exact: true }))
    expect(invoke.mock.calls.filter(([command]) => command === 'set_desktop_remote_access_enabled')).toHaveLength(0)
    await user.click(screen.getByRole('button', { name: 'Save' }))
    await user.click(screen.getByRole('button', { name: 'Enable', exact: true }))
    await screen.findByText('Settings changed. Reload before saving.')
    expect(invoke).toHaveBeenCalledWith('set_desktop_remote_access_enabled', {
        enabled: true, confirmedWarning: true, expectedRevision: 'core-1',
    })
    expect(toggle).toBeChecked()
    await user.click(screen.getByRole('button', { name: 'Discard' }))
    await waitFor(() => expect(toggle).not.toBeChecked())
    expect(screen.queryByText('Settings changed. Reload before saving.')).toBeNull()
})

it('refreshes native settings on live changes and keeps a dirty draft with its original revision', async () => {
    const user = userEvent.setup()
    let current = {
        revision: 'core-1', remote_access_enabled: false, bind_host: '127.0.0.1',
        server_url: 'http://127.0.0.1:45678/', requires_restart: false,
        remote_access_warning: 'Other devices can reach this server.',
    }
    const invoke = vi.fn().mockImplementation((command: string) => command === 'desktop_server_settings'
        ? Promise.resolve(current) : Promise.reject(new Error('Conflict')))
    window.__TAURI__ = { core: { invoke } }
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    await user.click(screen.getByRole('tab', { name: 'System' }))
    const toggle = await screen.findByRole('switch', { name: 'Remote desktop server access' })
    current = { ...current, revision: 'core-2', remote_access_enabled: true }
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await waitFor(() => expect(toggle).toBeChecked())
    await user.click(toggle)
    current = { ...current, revision: 'core-3' }
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText(/Desktop settings changed elsewhere/)
    expect(toggle).not.toBeChecked()
    await user.click(screen.getByRole('button', { name: 'Save' }))
    await screen.findByText('Conflict')
    expect(invoke).toHaveBeenCalledWith('set_desktop_remote_access_enabled', {
        enabled: false, confirmedWarning: false, expectedRevision: 'core-2',
    })
    await user.click(screen.getByRole('button', { name: 'Discard' }))
    await waitFor(() => expect(toggle).toBeChecked())
    expect(screen.queryByText(/Desktop settings changed elsewhere/)).toBeNull()
})

it('shows the native section while loading, reports initial failure, and retries', async () => {
    const user = userEvent.setup()
    let reject!: (error: Error) => void
    const invoke = vi.fn().mockImplementation((command: string) => command === 'desktop_server_settings'
        ? new Promise((_, fail) => { reject = fail }) : Promise.reject(new Error('Unsupported')))
    window.__TAURI__ = { core: { invoke } }
    render(<DialogProvider><SettingsPanel /></DialogProvider>)
    await user.click(screen.getByRole('tab', { name: 'System' }))
    expect(screen.getByRole('heading', { name: 'Desktop Server' })).toBeVisible()
    expect(screen.getByText('Loading Desktop settings…')).toBeVisible()
    await act(async () => reject(new Error('Native load failed')))
    expect(screen.getByText('Native load failed').closest('[role="alert"]')).not.toBeNull()
    invoke.mockResolvedValue({ revision: 'retry', remote_access_enabled: false, bind_host: '127.0.0.1', server_url: 'http://localhost', requires_restart: true, remote_access_warning: '' })
    await user.click(screen.getByRole('button', { name: 'Retry Desktop settings' }))
    expect(await screen.findByRole('switch', { name: 'Remote desktop server access' })).not.toBeChecked()
    expect(screen.getByText(/Restart Spark Desktop/)).toBeVisible()
    expect(screen.queryByText('Native load failed')).toBeNull()
})
