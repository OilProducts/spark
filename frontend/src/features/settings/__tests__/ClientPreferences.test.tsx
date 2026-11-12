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

it('validates preferences and retains the draft and original revision after live conflicts', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'structured', editor_sidebar_width: 288 },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const width = await screen.findByLabelText('Flow editor panel width')
    await user.clear(width)
    await user.type(width, '12')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.clear(width)
    await user.type(width, '400')
    const other = { ...view, revision: 'other', stored: { ...view.stored, editor_sidebar_width: 500 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(other)
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
    vi.mocked(saveClientPreferences).mockRejectedValue(new Error('Conflict: reload before saving.'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Conflict: reload before saving.')
    expect(width).toHaveValue(400)
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, editor_sidebar_width: 400 })
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(width).toHaveValue(500)
})

it('Reset layout clears dragged sizes in the draft, and Save applies it', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: null, editor_sidebar_width: 400, run_presentation: { sort: 'oldest', graph_height: 700 } },
        effective: { editor_mode: 'structured', editor_sidebar_width: 400, run_presentation: { sort: 'oldest', graph_height: 700 } } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockImplementation(async (_, preferences) => ({ ...view, revision: 'second', stored: preferences }))
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const width = await screen.findByLabelText('Flow editor panel width')
    const height = screen.getByLabelText('Run graph height')
    expect(width).toHaveValue(400)
    expect(height).toHaveValue(700)
    await user.click(screen.getByRole('button', { name: 'Reset layout' }))
    expect(width).toHaveValue(null)
    expect(height).toHaveValue(null)
    expect(screen.getByRole('button', { name: 'Reset layout' })).toBeDisabled()
    expect(saveClientPreferences).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, editor_sidebar_width: null, run_presentation: { sort: 'oldest', graph_height: null } })
})

it('defaults Show thinking on and offers it in the settings editor', async () => {
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
    const select = await screen.findByLabelText('Show thinking')
    expect(select).toHaveValue('')
    expect(select).toHaveDisplayValue('Default (on)')
})
