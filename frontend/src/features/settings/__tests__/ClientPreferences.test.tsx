import { act, cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { ClientPreferencesEditor } from '../ClientPreferencesEditor'
import { fetchClientPreferences, parseClientPreferences, saveClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'

vi.mock('../services/clientPreferences', async (original) => ({ ...await original<typeof import('../services/clientPreferences')>(), fetchClientPreferences: vi.fn(), saveClientPreferences: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })

it('reads an unknown stored editor mode, such as the retired raw mode, as structured', () => {
    const parse = (mode: unknown) => parseClientPreferences({ preferences: { scope: 'client', client_id: 'c', revision: 'r',
        stored: { editor_mode: mode, editor_sidebar_width: null }, effective: { editor_mode: mode, editor_sidebar_width: 288 } } }, '/workspace/api/settings')
    expect(parse('raw')).toMatchObject({ stored: { editor_mode: null }, effective: { editor_mode: 'structured' } })
    expect(parse('files')).toMatchObject({ stored: { editor_mode: 'files' }, effective: { editor_mode: 'files' } })
})

it('retains the draft and original revision after live conflicts, and reloads on Discard', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'structured', editor_sidebar_width: 288 },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const mode = await screen.findByLabelText('Editor mode')
    await user.selectOptions(mode, 'files')
    const other = { ...view, revision: 'other', stored: { ...view.stored, editor_mode: 'structured' as const, show_advanced_controls: true } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(other)
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
    vi.mocked(saveClientPreferences).mockRejectedValue(new Error('Conflict: reload before saving.'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Conflict: reload before saving.')
    expect(mode).toHaveValue('files')
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, editor_mode: 'files' })
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(mode).toHaveValue('structured')
    expect(screen.getByLabelText('Show advanced controls')).toBeChecked()
})

it('Reset layout clears dragged sizes in the draft, and Save applies it', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: null, editor_sidebar_width: 400, home_sidebar_primary_split_ratio: 0.4, run_presentation: { sort: 'oldest', graph_height: 700 } },
        effective: { editor_mode: 'structured', editor_sidebar_width: 400, home_sidebar_primary_split_ratio: 0.4, run_presentation: { sort: 'oldest', graph_height: 700 } } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockImplementation(async (_, preferences) => ({ ...view, revision: 'second', stored: preferences }))
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const reset = await screen.findByRole('button', { name: 'Reset layout' })
    expect(reset).toBeEnabled()
    await user.click(reset)
    expect(reset).toBeDisabled()
    expect(saveClientPreferences).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, editor_sidebar_width: null, home_sidebar_primary_split_ratio: null, run_presentation: { sort: 'oldest', graph_height: null } })
})

it('defaults Show thinking on and toggles it as a switch in the settings editor', async () => {
    const view = parseClientPreferences({ preferences: { scope: 'client', client_id: 'c', revision: 'r',
        stored: { editor_mode: null, editor_sidebar_width: null, show_thinking: false },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288, show_thinking: false } } }, '/workspace/api/settings')
    expect(view.stored.show_thinking).toBe(false)
    expect(() => parseClientPreferences({ preferences: { scope: 'client', client_id: 'c', revision: 'r',
        stored: { editor_mode: null, editor_sidebar_width: null, show_thinking: 'yes' },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288 } } }, '/workspace/api/settings')).toThrow('Invalid show_thinking')
    vi.mocked(fetchClientPreferences).mockResolvedValue({ client_id: 'c', revision: 'r',
        stored: { editor_mode: null, editor_sidebar_width: null }, effective: { editor_mode: 'structured', editor_sidebar_width: 288, show_thinking: true } })
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const toggle = await screen.findByRole('switch', { name: 'Show thinking' })
    expect(toggle).toBeChecked()
    await userEvent.setup().click(toggle)
    expect(toggle).not.toBeChecked()
})
