import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { LlmProfilesEditor, ExecutionProfilesEditor } from '../ProfileSettingsEditors'
import { profileSettingsRequest } from '../services/profileSettings'

vi.mock('../services/profileSettings', async (original) => ({ ...await original<object>(), profileSettingsRequest: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })
const llm = { revision: 'one', stored: [{ id: 'team', provider: 'openai_compatible', base_url: 'http://localhost/v1', models: ['model'] }], credential_status: { team: 'missing' } }
const execution = { revision: 'one', stored: { default_execution_profile_id: null, profiles: [{ id: 'native', label: 'Native', mode: 'native', enabled: true, capabilities: [], metadata: {} }] } }

it('requires Save, protects pending requests, retains failed drafts and reloads on Discard', async () => {
    vi.mocked(profileSettingsRequest).mockResolvedValue(llm)
    const user = userEvent.setup()
    render(<DialogProvider><LlmProfilesEditor /></DialogProvider>)
    const endpoint = await screen.findByLabelText('Endpoint')
    await user.clear(endpoint); await user.type(endpoint, 'http://localhost:1234/v1')
    expect(profileSettingsRequest).toHaveBeenCalledTimes(1)
    let reject!: (reason: Error) => void
    vi.mocked(profileSettingsRequest).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail }))
    const save = screen.getByRole('button', { name: 'Save LLM profiles' })
    await user.click(save)
    expect(save).toBeDisabled(); expect(endpoint).toBeDisabled()
    await act(async () => reject(new Error('Concurrent profile update.')))
    await screen.findByText('Concurrent profile update.')
    expect(endpoint).toHaveValue('http://localhost:1234/v1')
    expect(profileSettingsRequest).toHaveBeenLastCalledWith('llm_profiles', expect.any(Function), { revision: 'one', value: [{ ...llm.stored[0], base_url: 'http://localhost:1234/v1' }] })
    await user.click(screen.getByRole('button', { name: 'Discard LLM profile changes' }))
    await waitFor(() => expect(endpoint).toHaveValue('http://localhost/v1'))
})

it('blocks embedded credentials, adds and deletes draft profiles without writing', async () => {
    vi.mocked(profileSettingsRequest).mockResolvedValue(llm)
    const user = userEvent.setup()
    render(<DialogProvider><LlmProfilesEditor /></DialogProvider>)
    const endpoint = await screen.findByLabelText('Endpoint')
    await user.clear(endpoint); await user.type(endpoint, 'https://user:secret@example.com')
    expect(screen.getByRole('button', { name: 'Save LLM profiles' })).toBeDisabled()
    await user.click(screen.getByRole('button', { name: 'Add LLM profile' }))
    expect(screen.getAllByLabelText('Endpoint')).toHaveLength(2)
    await user.click(screen.getByRole('button', { name: 'Delete LLM profile team' }))
    expect(screen.getAllByLabelText('Endpoint')).toHaveLength(1)
    expect(profileSettingsRequest).toHaveBeenCalledTimes(1)
})

it('retains invalid metadata through live changes and failed discard, with navigation protection', async () => {
    vi.mocked(profileSettingsRequest).mockResolvedValue(execution)
    const user = userEvent.setup()
    render(<DialogProvider><ExecutionProfilesEditor /></DialogProvider>)
    const metadata = await screen.findByLabelText('Metadata (JSON object)')
    await user.clear(metadata); await user.type(metadata, 'broken')
    expect(screen.getByRole('button', { name: 'Save execution profiles' })).toBeDisabled()
    vi.mocked(profileSettingsRequest).mockResolvedValue({ ...execution, revision: 'two' })
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText(/Profiles changed elsewhere/)
    expect(metadata).toHaveValue('broken')
    const event = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(true)
    vi.mocked(profileSettingsRequest).mockRejectedValueOnce(new Error('Offline'))
    await user.click(screen.getByRole('button', { name: 'Discard execution profile changes' }))
    await screen.findByText('Offline')
    expect(metadata).toHaveValue('broken')
    await user.click(screen.getByRole('button', { name: 'Discard execution profile changes' }))
    await waitFor(() => expect(screen.getByLabelText('Metadata (JSON object)')).toHaveValue('{}'))
})
