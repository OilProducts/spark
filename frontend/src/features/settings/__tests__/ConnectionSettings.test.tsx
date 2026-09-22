import { act, cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchConnectionSettings, saveConnectionSettings, type ConnectionSettingsView } from '@/lib/api/settingsApi'
import { ConnectionSettingsEditor } from '../ConnectionSettingsEditor'

vi.mock('@/lib/api/settingsApi', () => ({ fetchConnectionSettings: vi.fn(), saveConnectionSettings: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })
const view: ConnectionSettingsView = { revision: 'one',
    stored: { server_host: '127.0.0.1', server_port: 8000, client_api_base_url: null },
    effective: { server_host: '0.0.0.0', server_port: 9000, client_api_base_url: 'http://127.0.0.1:8000' } }
it('validates fields, saves explicitly, and retains conflicts until Discard', async () => {
    vi.mocked(fetchConnectionSettings).mockResolvedValue(view)
    vi.mocked(saveConnectionSettings).mockRejectedValue(new Error('Settings changed elsewhere.'))
    render(<DialogProvider><ConnectionSettingsEditor /></DialogProvider>)
    const user = userEvent.setup()
    const target = await screen.findByLabelText('Client API target')
    await user.type(target, 'https://user:secret@example.com')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.clear(target)
    await user.type(target, 'https://example.com')
    expect(saveConnectionSettings).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Settings changed elsewhere.')
    expect(target).toHaveValue('https://example.com')
    expect(saveConnectionSettings).toHaveBeenCalledWith('one', { ...view.stored, client_api_base_url: 'https://example.com' })
    vi.mocked(fetchConnectionSettings).mockResolvedValue({ ...view, revision: 'two' })
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText(/Your draft is retained/)
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(target).toHaveValue('')
})
