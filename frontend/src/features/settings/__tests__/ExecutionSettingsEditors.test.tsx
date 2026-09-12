import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchProviderSettings, saveProviderSettings, fetchAgentSettings, saveAgentSettings } from '../services/executionSettings'
import { ProviderSettingsEditor } from '../ProviderSettingsEditor'
import { AgentSettingsEditor } from '../AgentSettingsEditor'

vi.mock('../services/executionSettings', async (original) => ({ ...await original<typeof import('../services/executionSettings')>(), fetchProviderSettings: vi.fn(), saveProviderSettings: vi.fn(), fetchAgentSettings: vi.fn(), saveAgentSettings: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })
it('validates provider references and endpoints, saves explicitly, and retains failed drafts', async () => {
    vi.mocked(fetchProviderSettings).mockResolvedValue({ revision: 'one', stored: {}, effective: { openai: { api_key_env: 'OPENAI_API_KEY' } }, credential_status: { openai: false } })
    vi.mocked(saveProviderSettings).mockRejectedValue(new Error('Conflict'))
    render(<DialogProvider><ProviderSettingsEditor /></DialogProvider>)
    const user = userEvent.setup()
    const group = await screen.findByRole('group', { name: 'openai' })
    const reference = within(group).getByLabelText('Credential environment variable')
    await user.type(reference, 'not a reference')
    expect(screen.getByRole('button', { name: 'Save provider connections' })).toBeDisabled()
    await user.clear(reference)
    await user.type(reference, 'TEAM_KEY')
    const endpoint = within(group).getByLabelText('base url')
    await user.type(endpoint, 'https://user:secret@example.com')
    expect(screen.getByRole('button', { name: 'Save provider connections' })).toBeDisabled()
    await user.clear(endpoint)
    await user.type(endpoint, 'https://example.com')
    expect(saveProviderSettings).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: 'Save provider connections' }))
    await screen.findByText('Conflict')
    expect(reference).toHaveValue('TEAM_KEY')
    expect(saveProviderSettings).toHaveBeenCalledWith('one', { openai: { api_key_env: 'TEAM_KEY', base_url: 'https://example.com' } })
    await user.click(screen.getByRole('button', { name: 'Discard provider changes' }))
    expect(reference).toHaveValue('')
})
it('validates session limits and saves with the section revision', async () => {
    const stored = { max_turns: 0, max_tool_rounds_per_input: 0, default_command_timeout_ms: 10000, max_command_timeout_ms: 600000, tool_output_limits: {}, line_limits: {}, enable_loop_detection: true, loop_detection_window: 10, max_subagent_depth: 1 }
    vi.mocked(fetchAgentSettings).mockResolvedValue({ revision: 'one', stored, effective: stored })
    vi.mocked(saveAgentSettings).mockImplementation(async (_, value) => ({ revision: 'two', stored: value, effective: value }))
    render(<DialogProvider><AgentSettingsEditor /></DialogProvider>)
    const user = userEvent.setup()
    const timeout = await screen.findByLabelText('default command timeout ms')
    await user.clear(timeout)
    expect(screen.getByRole('button', { name: 'Save agent settings' })).toBeDisabled()
    await user.type(timeout, '500')
    expect(saveAgentSettings).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: 'Save agent settings' }))
    await screen.findByRole('status')
    expect(saveAgentSettings).toHaveBeenCalledWith('one', { ...stored, default_command_timeout_ms: 500 })
})
