import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useEffect } from 'react'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { HarnessSettings } from '../HarnessSettings'
import { fetchAgentSettings, fetchHarnessCatalog, HIDDEN_ENV_VALUE, saveAcpHarness, type AgentSettingsView } from '../services/executionSettings'

vi.mock('../services/executionSettings', async (original) => ({ ...await original<typeof import('../services/executionSettings')>(),
    fetchAgentSettings: vi.fn(), fetchHarnessCatalog: vi.fn(), saveAcpHarness: vi.fn(), saveAgentSettings: vi.fn() }))
const { SignIn } = vi.hoisted(() => ({ SignIn: ({ onStatus }: { onStatus?: (state: 'connected') => void }) => {
    useEffect(() => onStatus?.('connected'), [onStatus])
    return <p>sign-in controls</p>
} }))
vi.mock('../CodexConnectionSettings', () => ({ CodexConnectionControls: SignIn }))
vi.mock('../ClaudeCodeConnectionSettings', () => ({ ClaudeCodeConnectionControls: SignIn }))
afterEach(() => { cleanup(); vi.resetAllMocks() })

const limits = { max_turns: 0, max_tool_rounds_per_input: 0, default_command_timeout_ms: 1000, max_command_timeout_ms: 1000, tool_output_limits: {}, line_limits: {}, enable_loop_detection: true, loop_detection_window: 10, max_subagent_depth: 1 }
const view: AgentSettingsView = {
    revision: 'one', stored: { ...limits, native: { claude_binary: '/missing/claude' } }, effective: { ...limits, native: { claude_binary: '/missing/claude' } },
    acp_harnesses: {
        helper: { registration: { name: 'Helper', executable: '/bin/helper', args: ['--acp'], env: { API_TOKEN: HIDDEN_ENV_VALUE } }, error: null },
        broken: { registration: { name: 'Broken', executable: '/bin/broken', env: { SPARK_HOME: HIDDEN_ENV_VALUE } }, error: 'SPARK_HOME is bound by Spark and cannot be overridden.' },
    },
}
const supported = { state: 'supported', reason: 'Works.' } as const

it('shows each harness with its status and next step, and edits an ACP harness without revealing or losing hidden values', async () => {
    vi.mocked(fetchAgentSettings).mockResolvedValue(view)
    vi.mocked(fetchHarnessCatalog).mockResolvedValue({ models: [], providers: {
        codex: { status: 'available', error: null, setup_status: 'ready' },
        'claude-code': { status: 'unavailable', error: 'claude code executable not found: /missing/claude', setup_status: 'not_installed' },
        'acp:helper': { status: 'unavailable', error: 'the harness requires authentication', setup_status: 'unauthenticated' },
        'acp:broken': { status: 'unavailable', error: 'misconfigured', setup_status: 'misconfigured' },
    } })
    vi.mocked(saveAcpHarness).mockResolvedValue(view)
    const user = userEvent.setup()
    render(<DialogProvider><HarnessSettings /></DialogProvider>)

    const codex = await screen.findByRole('group', { name: 'Codex' })
    expect(await within(codex).findByText('Ready')).toBeInTheDocument()
    expect(within(codex).getAllByText('Yes')).toHaveLength(2)
    const claude = screen.getByRole('group', { name: 'Claude Code' })
    expect(within(claude).getByText('Not installed')).toBeInTheDocument()
    expect(within(claude).getByText(/set Program below to the full path/)).toBeInTheDocument()
    // A missing program has nothing to sign in to.
    expect(within(claude).queryByText('sign-in controls')).not.toBeInTheDocument()
    expect(within(screen.getByRole('group', { name: 'Broken' })).getByText('Misconfigured')).toBeInTheDocument()
    const helper = screen.getByRole('group', { name: 'Helper' })
    expect(within(helper).getByText('Signed out')).toBeInTheDocument()
    expect(within(helper).getByText('Environment: API_TOKEN (values hidden)')).toBeInTheDocument()
    expect(screen.queryByText(HIDDEN_ENV_VALUE)).not.toBeInTheDocument()

    await user.click(within(helper).getByRole('button', { name: 'Edit Helper' }))
    const form = screen.getByRole('group', { name: 'Edit Helper' })
    expect(within(form).getByText('Value hidden')).toBeInTheDocument()
    await user.clear(within(form).getByLabelText('Name', { exact: true }))
    await user.type(within(form).getByLabelText('Name', { exact: true }), 'Helper Two')
    await user.click(within(form).getByRole('button', { name: 'Save' }))
    expect(saveAcpHarness).toHaveBeenLastCalledWith('one', 'helper', expect.objectContaining({ name: 'Helper Two', args: ['--acp'], env: { API_TOKEN: HIDDEN_ENV_VALUE } }))

    await user.click(within(await screen.findByRole('group', { name: 'Helper' })).getByRole('button', { name: 'Edit Helper' }))
    const again = screen.getByRole('group', { name: 'Edit Helper' })
    await user.click(within(again).getByRole('button', { name: 'Replace value of API_TOKEN' }))
    expect(within(again).getByLabelText('Value of API_TOKEN')).toHaveAttribute('type', 'password')
    await user.type(within(again).getByLabelText('Value of API_TOKEN'), 'new-token')
    await user.click(within(again).getByRole('button', { name: 'Save' }))
    expect(saveAcpHarness).toHaveBeenLastCalledWith('one', 'helper', expect.objectContaining({ env: { API_TOKEN: 'new-token' } }))
})

it('adds an ACP harness only once its ID, name and program are valid, and removes one after confirmation', async () => {
    vi.mocked(fetchAgentSettings).mockResolvedValue(view)
    vi.mocked(fetchHarnessCatalog).mockResolvedValue({ models: [], providers: { codex: { status: 'available', error: null, setup_status: 'ready', chat: supported, flow: supported },
        'acp:helper': { status: 'available', error: null, setup_status: 'ready', chat: { state: 'unsupported', reason: 'Chat pins instructions.' }, flow: supported,
            capabilities: { pinned_instructions: { state: 'unsupported', reason: 'No channel.' } } } } })
    vi.mocked(saveAcpHarness).mockResolvedValue(view)
    const user = userEvent.setup()
    render(<DialogProvider><HarnessSettings /></DialogProvider>)

    const helper = await screen.findByRole('group', { name: 'Helper' })
    expect(await within(helper).findByText(/To use it in chat:.*set Pinned instructions/)).toBeInTheDocument()

    await user.click(await screen.findByRole('button', { name: 'Add ACP harness' }))
    const form = screen.getByRole('group', { name: 'Add ACP harness' })
    await user.click(within(form).getByRole('button', { name: 'Add harness' }))
    expect(within(form).getByText('Enter a name.')).toBeInTheDocument()
    await user.type(within(form).getByLabelText('ID'), 'helper')
    expect(within(form).getByText('Another harness already uses this ID.')).toBeInTheDocument()
    await user.clear(within(form).getByLabelText('ID'))
    await user.type(within(form).getByLabelText('ID'), 'mine')
    await user.type(within(form).getByLabelText('Name', { exact: true }), 'Mine')
    await user.type(within(form).getByLabelText('Program'), 'my-agent')
    await user.click(within(form).getByRole('button', { name: 'Add variable' }))
    await user.type(within(form).getByLabelText('Name of environment variable 1'), 'SPARK_HOME')
    expect(within(form).getByText('SPARK_HOME is set by Spark and can’t be changed.')).toBeInTheDocument()
    expect(saveAcpHarness).not.toHaveBeenCalled()
    await user.click(within(form).getByRole('button', { name: 'Remove SPARK_HOME' }))
    await user.click(within(form).getByRole('button', { name: 'Add harness' }))
    expect(saveAcpHarness).toHaveBeenLastCalledWith('one', 'mine', expect.objectContaining({ name: 'Mine', executable: 'my-agent', env: {} }))

    await user.click(await screen.findByRole('button', { name: 'Remove Broken' }))
    await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Remove' }))
    expect(saveAcpHarness).toHaveBeenLastCalledWith('one', 'broken', null)
})
