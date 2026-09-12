import { act, cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { ClientPreferencesEditor } from '../ClientPreferencesEditor'
import { fetchClientPreferences, saveClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'

vi.mock('../services/clientPreferences', async (original) => ({ ...await original<typeof import('../services/clientPreferences')>(), fetchClientPreferences: vi.fn(), saveClientPreferences: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })

it('validates preferences and retains the draft and original revision after live conflicts', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'structured', editor_sidebar_width: 288 },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const width = await screen.findByLabelText('Editor sidebar width (pixels)')
    await user.clear(width)
    await user.type(width, '12')
    expect(screen.getByRole('button', { name: 'Save preferences' })).toBeDisabled()
    await user.clear(width)
    await user.type(width, '400')
    const other = { ...view, revision: 'other', stored: { ...view.stored, editor_sidebar_width: 500 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(other)
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
    vi.mocked(saveClientPreferences).mockRejectedValue(new Error('Conflict: reload before saving.'))
    await user.click(screen.getByRole('button', { name: 'Save preferences' }))
    await screen.findByText('Conflict: reload before saving.')
    expect(width).toHaveValue(400)
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, editor_sidebar_width: 400 })
    await user.click(screen.getByRole('button', { name: 'Discard preference changes' }))
    expect(width).toHaveValue(500)
})

it('validates and explicitly saves the home split, with Discard restoring the document', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: null, editor_sidebar_width: null, home_sidebar_primary_split_ratio: 0.5 },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288, home_sidebar_primary_split_ratio: 0.5 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockImplementation(async (_, preferences) => ({ ...view, revision: 'second', stored: preferences }))
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const split = await screen.findByLabelText('Home sidebar primary split (0–1)')
    await user.clear(split)
    await user.type(split, '1.5')
    expect(screen.getByRole('button', { name: 'Save preferences' })).toBeDisabled()
    expect(screen.getByText('Choose a number from 0 to 1.')).toBeVisible()
    await user.clear(split)
    await user.type(split, '0.7')
    expect(saveClientPreferences).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: 'Discard preference changes' }))
    expect(split).toHaveValue(0.5)
    await user.clear(split)
    await user.type(split, '0.6')
    await user.click(screen.getByRole('button', { name: 'Save preferences' }))
    await screen.findByText('Client preferences saved.')
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, home_sidebar_primary_split_ratio: 0.6 })
})
