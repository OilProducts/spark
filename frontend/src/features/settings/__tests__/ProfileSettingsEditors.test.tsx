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
    await user.click(await screen.findByText('team · missing'))
    const endpoint = await screen.findByLabelText('Endpoint')
    await user.clear(endpoint); await user.type(endpoint, 'http://localhost:1234/v1')
    expect(profileSettingsRequest).toHaveBeenCalledTimes(1)
    let reject!: (reason: Error) => void
    vi.mocked(profileSettingsRequest).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail }))
    const save = screen.getByRole('button', { name: /^Save/ })
    await user.click(save)
    expect(save).toBeDisabled(); expect(endpoint).toBeDisabled()
    await act(async () => reject(new Error('Concurrent profile update.')))
    await screen.findByText('Concurrent profile update.')
    expect(endpoint).toHaveValue('http://localhost:1234/v1')
    expect(profileSettingsRequest).toHaveBeenLastCalledWith('llm_profiles', expect.any(Function), { revision: 'one', value: [{ ...llm.stored[0], base_url: 'http://localhost:1234/v1' }] })
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(endpoint).toHaveValue('http://localhost/v1'))
})

it('blocks embedded credentials, adds and deletes draft profiles without writing', async () => {
    vi.mocked(profileSettingsRequest).mockResolvedValue(llm)
    const user = userEvent.setup()
    render(<DialogProvider><LlmProfilesEditor /></DialogProvider>)
    await user.click(await screen.findByText('team · missing'))
    const endpoint = await screen.findByLabelText('Endpoint')
    await user.clear(endpoint); await user.type(endpoint, 'https://user:secret@example.com')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
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
    await user.click(await screen.findByText(/Native ·/))
    await user.click(screen.getByText('Advanced'))
    const metadata = await screen.findByLabelText('Metadata (JSON object)')
    await user.clear(metadata); await user.type(metadata, 'broken')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    vi.mocked(profileSettingsRequest).mockResolvedValue({ ...execution, revision: 'two' })
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText(/Profiles changed elsewhere/)
    expect(metadata).toHaveValue('broken')
    expect(screen.getByRole('button', { name: 'Delete execution profile native' })).toBeDisabled()
    expect(screen.getByText(/Fix invalid metadata JSON/)).toBeVisible()
    const event = new Event('beforeunload', { cancelable: true })
    window.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(true)
    vi.mocked(profileSettingsRequest).mockRejectedValueOnce(new Error('Offline'))
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await screen.findByText('Offline')
    expect(metadata).toHaveValue('broken')
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(screen.getByLabelText('Metadata (JSON object)')).toHaveValue('{}'))
})

it('keeps container drafts across mode changes and blocks all deletes for another profile’s invalid JSON', async () => {
    vi.mocked(profileSettingsRequest).mockResolvedValue({ ...execution, stored: { ...execution.stored, profiles: [
        execution.stored.profiles[0], { ...execution.stored.profiles[0], id: 'container', label: 'Container', mode: 'local_container', image: 'worker:old', metadata: { 'container.mounts': ['/tmp:/work'] } },
    ] } })
    const user = userEvent.setup()
    render(<DialogProvider><ExecutionProfilesEditor /></DialogProvider>)
    const summary = await screen.findByText(/Container ·/)
    expect(summary.closest('details')).not.toHaveAttribute('open')
    await user.click(summary)
    const image = screen.getAllByLabelText('Container image')[1]
    const mounts = screen.getAllByLabelText('Mounts (host:container[:options], one per line)')[1]
    const mode = screen.getAllByLabelText('Mode')[1]
    await user.clear(image); await user.type(image, 'worker:draft')
    await user.clear(mounts); await user.type(mounts, '/draft:/work')
    await user.selectOptions(mode, 'native')
    expect(image).not.toBeVisible(); expect(mounts).not.toBeVisible()
    await user.selectOptions(mode, 'local_container')
    expect(image).toHaveValue('worker:draft'); expect(mounts).toHaveValue('/draft:/work')
    await user.click(screen.getAllByText('Advanced')[1])
    const metadata = screen.getAllByLabelText('Metadata (JSON object)')[1]
    await user.clear(metadata); await user.type(metadata, 'invalid')
    expect(metadata).toHaveAccessibleDescription('Enter a valid JSON object.')
    await user.click(screen.getByText(/Native ·/))
    for (const button of screen.getAllByRole('button', { name: /Delete execution profile/ })) expect(button).toBeDisabled()
    expect(screen.getByText(/Fix invalid metadata JSON/)).toBeVisible()
    await user.clear(metadata); await user.type(metadata, '{{}')
    for (const button of screen.getAllByRole('button', { name: /Delete execution profile/ })) expect(button).toBeEnabled()
    await user.click(screen.getByRole('button', { name: 'Add execution profile' }))
    expect(screen.getByText(/Execution profile 3/).closest('details')).toHaveAttribute('open')
})
