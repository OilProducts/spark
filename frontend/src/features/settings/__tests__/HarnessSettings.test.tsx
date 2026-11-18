import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useEffect } from 'react'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { HarnessSettings } from '../HarnessSettings'
import { cancelAcpSignIn, fetchAgentSettings, fetchHarnessCatalog, HIDDEN_ENV_VALUE, saveAcpHarness, signInAcpHarness, type AgentSettingsView } from '../services/executionSettings'

vi.mock('../services/executionSettings', async (original) => ({ ...await original<typeof import('../services/executionSettings')>(),
    fetchAgentSettings: vi.fn(), fetchHarnessCatalog: vi.fn(), saveAcpHarness: vi.fn(), saveAgentSettings: vi.fn(), signInAcpHarness: vi.fn(), cancelAcpSignIn: vi.fn() }))
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
    expect(screen.getByLabelText('Harness overview')).toHaveTextContent('1 ready · 1 signed out · 1 not installed · 1 misconfigured')
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

it('signs in to an ACP harness with a method it offers, then checks it again', async () => {
    vi.mocked(fetchAgentSettings).mockResolvedValue(view)
    vi.mocked(fetchHarnessCatalog).mockResolvedValueOnce({ models: [], providers: {
        'acp:helper': { status: 'unavailable', error: 'the harness requires authentication: run helper login', setup_status: 'unauthenticated',
            auth_methods: [{ id: 'browser', name: 'Browser', description: 'Opens your browser.' }] },
    } }).mockResolvedValue({ models: [], providers: { 'acp:helper': { status: 'available', error: null, setup_status: 'ready', chat: { state: 'supported', reason: 'Yes.' }, flow: { state: 'supported', reason: 'Yes.' } } } })
    vi.mocked(signInAcpHarness).mockResolvedValue({ setup_status: 'ready', error: null })
    const user = userEvent.setup()
    render(<DialogProvider><HarnessSettings /></DialogProvider>)

    const helper = await screen.findByRole('group', { name: 'Helper' })
    expect(await within(helper).findByText(/run helper login/)).toBeInTheDocument()
    expect(within(helper).getByText('Opens your browser.')).toBeInTheDocument()
    await user.click(within(helper).getByRole('button', { name: 'Sign in to Helper with Browser' }))
    expect(signInAcpHarness).toHaveBeenCalledWith('helper', 'browser')
    expect(await within(helper).findByText('Ready')).toBeInTheDocument()
    // The sign-in replaces the kept check, as Check again does.
    expect(fetchHarnessCatalog).toHaveBeenCalledTimes(2)
    expect(fetchHarnessCatalog).toHaveBeenLastCalledWith(null, true)

    // A check that fails doesn't leave it showing as the last check found it.
    vi.mocked(fetchHarnessCatalog).mockRejectedValueOnce(new Error('agents.max_turns is invalid'))
    await user.click(within(helper).getByRole('button', { name: 'Check Helper again' }))
    expect(await within(helper).findByText('Not checked')).toBeInTheDocument()
})

it('shows a harness as checking until its check ends, then what it found without checking again', async () => {
    vi.mocked(fetchAgentSettings).mockResolvedValue(view)
    vi.mocked(fetchHarnessCatalog).mockResolvedValueOnce({ models: [], providers: {
        codex: { status: 'available', error: null, setup_status: 'ready' },
        'claude-code': { status: 'checking', error: null },
        'acp:helper': { status: 'checking', error: null },
    } }).mockResolvedValue({ models: [], providers: {
        codex: { status: 'available', error: null, setup_status: 'ready' },
        'claude-code': { status: 'unavailable', error: 'claude code executable not found: /missing/claude', setup_status: 'not_installed' },
        'acp:helper': { status: 'unavailable', error: 'Helper did not answer in time.', setup_status: 'unreachable' },
    } })
    render(<DialogProvider><HarnessSettings /></DialogProvider>)

    const codex = await screen.findByRole('group', { name: 'Codex' })
    expect(await within(codex).findByText('Ready')).toBeInTheDocument()
    const claude = screen.getByRole('group', { name: 'Claude Code' })
    expect(within(claude).getByText('Checking…')).toBeInTheDocument()
    // Sign-in is read once the program is checked.
    expect(within(claude).queryByText('sign-in controls')).not.toBeInTheDocument()
    const helper = screen.getByRole('group', { name: 'Helper' })
    expect(within(helper).getByText('Checking…')).toBeInTheDocument()

    act(() => window.dispatchEvent(new Event('spark:harness-checked')))
    expect(await within(helper).findByText('Not responding')).toBeInTheDocument()
    expect(within(helper).getByText('Helper did not answer in time.')).toBeInTheDocument()
    expect(within(claude).getByText('Not installed')).toBeInTheDocument()
    expect(within(codex).getByText('Ready')).toBeInTheDocument()
    expect(fetchHarnessCatalog).toHaveBeenCalledTimes(2)
    expect(fetchHarnessCatalog).toHaveBeenLastCalledWith(null, false)
})

it('keeps a misconfigured registration’s values in its form, and stops a save over a change made elsewhere', async () => {
    const wrong: AgentSettingsView = { ...view, acp_harnesses: {
        helper: view.acp_harnesses!.helper,
        wrong: { registration: { name: 'Wrong', executable: '/bin/wrong', args: '--acp --verbose' as unknown as string[] }, error: '`args` has the wrong type: expected a sequence, found string.' },
    } }
    vi.mocked(fetchAgentSettings).mockResolvedValue(wrong)
    vi.mocked(fetchHarnessCatalog).mockRejectedValue(new Error('agents.max_turns is invalid'))
    vi.mocked(saveAcpHarness).mockResolvedValue(wrong)
    const user = userEvent.setup()
    render(<DialogProvider><HarnessSettings /></DialogProvider>)

    // A failed check says so instead of checking forever.
    const codex = await screen.findByRole('group', { name: 'Codex' })
    expect(await within(codex).findByText('Not checked')).toBeInTheDocument()
    const row = screen.getByRole('group', { name: 'Wrong' })
    expect(within(row).getByText(/runs/)).toHaveTextContent('runs /bin/wrong --acp --verbose')
    await user.click(within(row).getByRole('button', { name: 'Edit Wrong' }))
    const form = screen.getByRole('group', { name: 'Edit Wrong' })
    expect(within(form).getByText(/`args` has the wrong type/)).toBeInTheDocument()
    expect(within(form).getByLabelText('Arguments')).toHaveValue('--acp\n--verbose')
    await user.click(within(form).getByRole('button', { name: 'Save' }))
    expect(saveAcpHarness).toHaveBeenLastCalledWith('one', 'wrong', expect.objectContaining({ args: ['--acp', '--verbose'] }))

    // Two editors stay open; one whose harness changed elsewhere can't save over it.
    await user.click(within(screen.getByRole('group', { name: 'Helper' })).getByRole('button', { name: 'Edit Helper' }))
    await user.click(screen.getByRole('button', { name: 'Add ACP harness' }))
    const edit = screen.getByRole('group', { name: 'Edit Helper' })
    await user.type(within(edit).getByLabelText('Name', { exact: true }), ' Two')
    const elsewhere: AgentSettingsView = { ...wrong, revision: 'two', acp_harnesses: { ...wrong.acp_harnesses,
        helper: { registration: { ...view.acp_harnesses!.helper.registration, env: { API_TOKEN: HIDDEN_ENV_VALUE, OTHER: HIDDEN_ENV_VALUE } }, error: null } } }
    vi.mocked(fetchAgentSettings).mockResolvedValue(elsewhere)
    act(() => { window.dispatchEvent(new CustomEvent('spark:settings-live-event', { detail: { payload: { section: 'agents', revision: 'two' } } })) })
    expect(await within(edit).findByText(/changed in another window/)).toBeInTheDocument()
    expect(within(edit).getByRole('button', { name: 'Save' })).toBeDisabled()
    expect(screen.getByRole('group', { name: 'Add ACP harness' })).toBeInTheDocument()
    await user.click(within(edit).getByRole('button', { name: 'Start over from what’s saved' }))
    await waitFor(() => expect(within(edit).queryByText(/changed in another window/)).not.toBeInTheDocument())
    expect(within(edit).getByLabelText('Name', { exact: true })).toHaveValue('Helper')
    expect(within(edit).getByLabelText('Name of environment variable 2')).toHaveValue('OTHER')
})

it('keeps what it can’t show, says what flows refuse, and leaves details as the user set them across checks', async () => {
    const odd: AgentSettingsView = { ...view, acp_harnesses: { helper: { registration: { name: 'Helper', executable: '/bin/helper', args: ['--prompt', 'line one\nline two', ''] }, error: null } } }
    vi.mocked(fetchAgentSettings).mockResolvedValue(odd)
    const ready = { models: [], providers: { 'acp:helper': { status: 'available' as const, error: null, setup_status: 'ready' as const, chat: supported,
        flow: { state: 'limited' as const, reason: 'A step that sets a model is refused.' }, capabilities: { model_selection: { state: 'unsupported' as const, reason: 'No choice.' } } } } }
    vi.mocked(fetchHarnessCatalog).mockResolvedValue(ready)
    vi.mocked(saveAcpHarness).mockResolvedValue(odd)
    const user = userEvent.setup()
    render(<DialogProvider><HarnessSettings /></DialogProvider>)

    const helper = await screen.findByRole('group', { name: 'Helper' })
    expect(await within(helper).findByText('Partly')).toBeInTheDocument()
    expect(within(helper).getByText(/A step that sets a model is refused/)).toBeInTheDocument()
    // Opened details stay open when a check finds it ready again.
    await user.click(within(helper).getByText('Details'))
    await user.click(within(helper).getByRole('button', { name: 'Check Helper again' }))
    await waitFor(() => expect(fetchHarnessCatalog).toHaveBeenCalledTimes(2))
    expect(await within(helper).findByText('Ready')).toBeInTheDocument()
    expect(within(helper).getByText('Details').closest('details')).toHaveAttribute('open')

    await user.click(within(helper).getByRole('button', { name: 'Edit Helper' }))
    const form = screen.getByRole('group', { name: 'Edit Helper' })
    await user.type(within(form).getByLabelText('Name', { exact: true }), ' Two')
    await user.click(within(form).getByRole('button', { name: 'Save' }))
    expect(saveAcpHarness).toHaveBeenLastCalledWith('one', 'helper', expect.objectContaining({ args: ['--prompt', 'line one\nline two', ''] }))
    // The save's message gives what the check found, not that it is still checking.
    expect(await screen.findByText('Saved Helper Two. Status: Ready.')).toBeInTheDocument()
})

it('shows a terminal sign-in as the command to run, and cancels a sign-in left unfinished', async () => {
    vi.mocked(fetchAgentSettings).mockResolvedValue(view)
    vi.mocked(fetchHarnessCatalog).mockResolvedValue({ models: [], providers: {
        'acp:helper': { status: 'unavailable', error: 'the harness requires authentication', setup_status: 'unauthenticated',
            auth_methods: [{ id: 'browser', name: 'Browser' }, { id: 'tui', name: 'Terminal login', terminal: ['MODE=device', "REGION=us east's", '/bin/helper', '--acp', 'X=log in'] }] },
    } })
    let finish = (_: { setup_status: string; error: string | null }) => {}
    vi.mocked(signInAcpHarness).mockReturnValue(new Promise((resolve) => { finish = resolve }))
    vi.mocked(cancelAcpSignIn).mockImplementation(async () => { finish({ setup_status: 'unauthenticated', error: 'sign-in was cancelled' }) })
    const user = userEvent.setup()
    render(<DialogProvider><HarnessSettings /></DialogProvider>)

    const helper = await screen.findByRole('group', { name: 'Helper' })
    expect(await within(helper).findByText("MODE=device REGION='us east'\\''s' /bin/helper --acp 'X=log in'")).toBeInTheDocument()
    expect(within(helper).getByRole('button', { name: 'Copy Terminal login command' })).toBeInTheDocument()
    expect(within(helper).getByText(/Spark starts this agent with API_TOKEN set/)).toBeInTheDocument()
    expect(within(screen.getByRole('navigation', { name: 'Harnesses to fix' })).getByRole('button', { name: 'Helper (signed out)' })).toBeInTheDocument()

    await user.click(within(helper).getByRole('button', { name: 'Sign in to Helper with Browser' }))
    expect(await within(helper).findByText('Signing in…')).toBeInTheDocument()
    await user.click(within(helper).getByRole('button', { name: 'Cancel sign-in' }))
    expect(cancelAcpSignIn).toHaveBeenCalledWith('helper')
    expect(await within(helper).findByText('Signed out')).toBeInTheDocument()
    expect(within(helper).queryByRole('alert')).not.toBeInTheDocument()
})
