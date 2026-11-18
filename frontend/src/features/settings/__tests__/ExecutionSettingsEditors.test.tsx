import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchProviderSettings, saveProviderSettings, fetchAgentSettings, saveAgentSettings, msToUnit, unitToMs, agentLimitError } from '../services/executionSettings'
import { ProviderSettingsEditor } from '../ProviderSettingsEditor'
import { AgentSettingsEditor } from '../AgentSettingsEditor'

vi.mock('../services/executionSettings', async (original) => ({ ...await original<typeof import('../services/executionSettings')>(), fetchProviderSettings: vi.fn(), saveProviderSettings: vi.fn(), fetchAgentSettings: vi.fn(), saveAgentSettings: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })
it('validates provider references and endpoints, saves explicitly, and retains failed drafts', async () => {
    vi.mocked(fetchProviderSettings).mockResolvedValue({ revision: 'one', stored: {}, effective: { openai: { api_key_env: 'OPENAI_API_KEY' } }, credential_status: { openai: false } })
    vi.mocked(saveProviderSettings).mockRejectedValue(new Error('Conflict'))
    render(<DialogProvider><ProviderSettingsEditor /></DialogProvider>)
    const user = userEvent.setup()
    await user.click(await screen.findByText('OpenAI · Missing credentials'))
    const group = screen.getByRole('group', { name: 'OpenAI' })
    const reference = within(group).getByLabelText('Credential environment variable')
    await user.type(reference, 'not a reference')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.clear(reference)
    await user.type(reference, 'TEAM_KEY')
    const endpoint = within(group).getByLabelText('Base URL')
    await user.type(endpoint, 'https://user:secret@example.com')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.clear(endpoint)
    await user.type(endpoint, 'https://example.com')
    expect(saveProviderSettings).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Conflict')
    expect(reference).toHaveValue('TEAM_KEY')
    expect(saveProviderSettings).toHaveBeenCalledWith('one', { openai: { api_key_env: 'TEAM_KEY', base_url: 'https://example.com' } })
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(reference).toHaveValue('')
})
it('validates session limits and saves with the section revision', async () => {
    const stored = { max_turns: 0, max_tool_rounds_per_input: 0, default_command_timeout_ms: 10000, max_command_timeout_ms: 600000, tool_output_limits: {}, line_limits: {}, enable_loop_detection: true, loop_detection_window: 10, max_subagent_depth: 1 }
    vi.mocked(fetchAgentSettings).mockResolvedValue({ revision: 'one', stored, effective: stored })
    vi.mocked(saveAgentSettings).mockImplementation(async (_, value) => ({ revision: 'two', stored: value, effective: value }))
    render(<DialogProvider><AgentSettingsEditor /></DialogProvider>)
    const user = userEvent.setup()
    // Shown in seconds and minutes, saved in milliseconds.
    const timeout = await screen.findByLabelText('Command time limit')
    expect(timeout).toHaveValue(10)
    expect(screen.getByLabelText('Longest command time')).toHaveValue(10)
    await user.clear(timeout)
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    expect(timeout).toHaveAccessibleDescription(expect.stringContaining('Enter a time longer than 0 seconds.'))
    await user.type(timeout, '900')
    expect(screen.getByLabelText('Longest command time')).toHaveAccessibleDescription(expect.stringContaining('Must be at least the command time limit (900 seconds).'))
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.clear(timeout)
    await user.type(timeout, '0.5')
    expect(saveAgentSettings).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByRole('status')
    expect(saveAgentSettings).toHaveBeenCalledWith('one', { ...stored, default_command_timeout_ms: 500 })
})

it('shows why unreadable session limits can’t be read and repairs them for saving', async () => {
    const repair = { max_turns: 0, max_tool_rounds_per_input: 7, default_command_timeout_ms: 10000, max_command_timeout_ms: 600000, tool_output_limits: {}, line_limits: {}, enable_loop_detection: true, loop_detection_window: 10, max_subagent_depth: 1 }
    vi.mocked(fetchAgentSettings).mockResolvedValue({ revision: 'one', stored: null, repair_defaults: repair, effective: null, validation_errors: ['Invalid agents section: `max_turns` has the wrong type.'] })
    vi.mocked(saveAgentSettings).mockImplementation(async (_, value) => ({ revision: 'two', stored: value, effective: value }))
    render(<DialogProvider><AgentSettingsEditor /></DialogProvider>)
    const user = userEvent.setup()
    expect(await screen.findByText(/`max_turns` has the wrong type/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Reset what can’t be read' }))
    expect(screen.getByLabelText('Tool rounds per message')).toHaveValue(7)
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    expect(saveAgentSettings).toHaveBeenCalledWith('one', repair)
})

it('collapses agent advanced groups, opens invalid paths and output limits, and keeps controls outside disclosures', async () => {
    const stored = { max_turns: 0, max_tool_rounds_per_input: 0, default_command_timeout_ms: 10000, max_command_timeout_ms: 600000, tool_output_limits: { shell: -1 }, line_limits: {}, enable_loop_detection: true, loop_detection_window: 10, max_subagent_depth: 1, native: { codex_runtime_root: ' ' } }
    vi.mocked(fetchAgentSettings).mockResolvedValue({ revision: 'one', stored, effective: stored })
    const user = userEvent.setup()
    render(<DialogProvider><AgentSettingsEditor /></DialogProvider>)
    const path = await screen.findByLabelText('Codex runtime root')
    expect(path).toBeVisible()
    expect(path).toHaveAccessibleDescription('Enter a nonempty path or leave blank for the default.')
    expect(screen.getByText('Codex and Claude paths').closest('details')).toHaveAttribute('open')
    expect(screen.getByText('Tool output limits').closest('details')).toHaveAttribute('open')
    for (const name of ['Permissions and environment', 'Debug logs']) expect(screen.getByText(name).closest('details')).not.toHaveAttribute('open')
    expect(screen.getByRole('button', { name: /^Save/ }).closest('details')).toBeNull()
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    await user.clear(path)
    const limit = screen.getByLabelText('shell')
    await user.clear(limit); await user.type(limit, '10')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeEnabled()
})

it('converts command time limits between milliseconds and the shown unit', () => {
    expect(msToUnit(10_000, 'seconds')).toBe(10)
    expect(msToUnit(1234, 'seconds')).toBe(1.234)
    expect(msToUnit(600_000, 'minutes')).toBe(10)
    expect(msToUnit(10_000, 'minutes')).toBe(0.167)
    expect(msToUnit(NaN, 'seconds')).toBe('')
    expect(unitToMs('1.5', 'seconds')).toBe(1500)
    expect(unitToMs('0.25', 'minutes')).toBe(15_000)
    expect(unitToMs('', 'minutes')).toBeNaN()
    const limits = { max_turns: 0, max_tool_rounds_per_input: 0, default_command_timeout_ms: 10_000, max_command_timeout_ms: 600_000, tool_output_limits: {}, line_limits: {}, enable_loop_detection: true, loop_detection_window: 10, max_subagent_depth: 1 }
    expect(agentLimitError(limits, 'max_command_timeout_ms')).toBe('')
    expect(agentLimitError({ ...limits, max_command_timeout_ms: 5000 }, 'max_command_timeout_ms')).toBe('Must be at least the command time limit (10 seconds).')
    expect(agentLimitError({ ...limits, default_command_timeout_ms: NaN }, 'default_command_timeout_ms')).toBe('Enter a time longer than 0 seconds.')
    expect(agentLimitError({ ...limits, max_turns: 1.5 }, 'max_turns')).toBe('Enter a whole number, 0 or more.')
    expect(agentLimitError({ ...limits, loop_detection_window: 0 }, 'loop_detection_window')).toBe('Enter at least 1 while loop warnings are on.')
    expect(agentLimitError({ ...limits, loop_detection_window: 0, enable_loop_detection: false }, 'loop_detection_window')).toBe('')
})
