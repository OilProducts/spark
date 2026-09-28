import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { MissionsPanel } from '../MissionsPanel'
import { useStore } from '@/store'

const fields = { title: 'Deliver search', description: '', archived: false }
type Task = { id: string; revision: number; fields: typeof fields; activity: unknown[]; status?: string; updated_at?: string; [key: string]: unknown }
let task: Task
let calls: { url: string; body: Record<string, unknown> }[]
let snapshot: Record<string, unknown>
beforeEach(() => {
    window.innerWidth = 1440
    task = { id: 'task-1', revision: 1, fields: { ...fields }, activity: [], status: 'draft', updated_at: '2026-09-20 10:00:00.0 +00:00:00' }
    calls = []
    snapshot = conversation([])
    useStore.setState({ activeProjectPath: '/project', viewMode: 'missions' })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        if (url.includes('/chat-models')) return { ok: true, json: async () => ({ models: [{ provider: 'claude-code', id: 'opus', display: 'Opus', is_default: true, supported_reasoning_efforts: ['high'] }], providers: { codex: { status: 'unavailable', error: null } } }) }
        if (init?.body) {
            const body = JSON.parse(String(init.body)); calls.push({ url, body })
            if (url.includes('/conversations/')) return { ok: true, json: async () => snapshot }
            if (init.method === 'PATCH' && body.revision !== task.revision) return { ok: false, status: 409, json: async () => ({ detail: 'Conflict' }) }
            if (init.method === 'POST' && !url.includes('/missions/')) task = { ...task, id: 'task-new', revision: 1, fields: { ...task.fields, ...body.fields } }
            else if (body.fields) task = { ...task, revision: task.revision + 1, fields: { ...task.fields, ...body.fields } }
            return { ok: true, json: async () => task }
        }
        if (url.includes('/conversations/')) return { ok: true, json: async () => snapshot }
        if (url === '/workspace/api/playbooks') return { ok: true, json: async () => [{ name: 'bug-report', title: 'Bug report', description: 'Fix a bug.' }] }
        return { ok: true, json: async () => ({ missions: [task] }) }
    }))
})
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })

function conversation(turns: unknown[]) {
    return { schema_version: 5, revision: turns.length, conversation_id: 'task-1', project_path: '/project', turns, segments: [], event_log: [], flow_run_requests: [], flow_launches: [], proposed_plans: [] }
}
const turn = (id: string, role: string, content: string, kind = 'message') => ({ id, role, content, kind, status: 'complete', timestamp: '2026-09-20T10:00:00Z' })
const detail = () => within(screen.getByRole('region', { name: 'Mission details' }))
const editor = detail
/** Opens the header menu and picks an action. */
function menu(action: string) {
    fireEvent.keyDown(detail().getByRole('button', { name: 'Mission actions' }), { key: 'Enter' })
    fireEvent.click(screen.getByRole('menuitem', { name: action }))
}
const editMission = () => menu('Edit')
const mission = (overrides: Partial<Task>) => ({ ...task, ...overrides, fields: { ...task.fields, ...(overrides.fields ?? {}) } })

it('groups missions by derived status, newest update first, hiding empty groups', async () => {
    const board = [
        mission({ id: 'a', status: 'running', updated_at: '2026-09-20 09:00:00.0 +00:00:00', fields: { ...fields, title: 'Older running' }, runs: [{ run_id: 'r1', flow_name: 'work/build.yaml', summary: 'Build', launched_at: 't', status: 'running' }] }),
        mission({ id: 'b', status: 'running', updated_at: '2026-09-21 09:00:00.0 +00:00:00', fields: { ...fields, title: 'Newer running' } }),
        mission({ id: 'c', status: 'needs_you', fields: { ...fields, title: 'Gated' }, runs: [{ run_id: 'r2', flow_name: 'work/review.yaml', summary: 'Review it', launched_at: 't', status: 'waiting' }] }),
        mission({ id: 'd', status: 'closed', fields: { ...fields, title: 'Finished' }, closed: { status: 'done', reason: 'Shipped', at: 't' } }),
    ]
    vi.mocked(fetch).mockImplementation(async () => ({ ok: true, json: async () => ({ missions: board }) }) as Response)
    render(<MissionsPanel active />)
    await screen.findByRole('button', { name: 'Gated' })
    expect(screen.getAllByRole('heading', { level: 2 }).map(heading => heading.firstChild?.textContent)).toEqual(['Needs you', 'Running', 'Closed'])
    const running = within(screen.getByRole('region', { name: 'Running' }))
    expect(running.getAllByRole('button').map(button => button.getAttribute('aria-label'))).toEqual(['Newer running', 'Older running'])
    expect(running.getByText('1 run in flight')).toBeInTheDocument()
    expect(running.getByText('Agent is working')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Gated' })).toHaveAccessibleDescription('Review it is waiting on a human gate')
    expect(within(screen.getByRole('region', { name: 'Closed' })).getByText('Closed as done: Shipped')).toBeInTheDocument()
    expect(screen.queryByRole('region', { name: 'Drafts' })).not.toBeInTheDocument()
    // Only closed rows offer Archive.
    expect(screen.getAllByRole('button', { name: /^Archive / }).map(button => button.getAttribute('aria-label'))).toEqual(['Archive Finished'])
})

it('archives closed rows and shows them again with Show archived', async () => {
    task = mission({ status: 'closed', closed: { status: 'done', reason: 'Shipped', at: 't' } })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Archive Deliver search' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Deliver search' })).not.toBeInTheDocument())
    expect(calls[0].body).toEqual({ revision: 1, fields: { archived: true }, actor: 'human' })
    fireEvent.click(screen.getByRole('button', { name: 'Show archived' }))
    expect(within(screen.getByRole('button', { name: 'Deliver search' })).getByText('Archived')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Restore Deliver search' }))
    await waitFor(() => expect(task.fields.archived).toBe(false))
})

it('shows a draft with its pinned objective and Start in place of the reply box', async () => {
    task.fields.description = 'Search should return matching documents.'
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(within(detail().getByRole('region', { name: 'Objective' })).getByText(task.fields.description)).toBeInTheDocument()
    expect(detail().queryByLabelText('Reply')).not.toBeInTheDocument()
    vi.mocked(fetch).mockImplementationOnce(async (url, init) => {
        calls.push({ url: String(url), body: JSON.parse(String(init?.body)) })
        task = mission({ status: 'running', conversation_id: 'task-1', started_at: 't' })
        return { ok: true, json: async () => task } as Response
    })
    fireEvent.click(detail().getByRole('button', { name: 'Start' }))
    expect(await detail().findByLabelText('Reply')).toBeInTheDocument()
    expect(calls[0].url).toBe('/workspace/api/missions/task-1/start?project_path=%2Fproject')
    expect(detail().getByRole('status')).toHaveTextContent('Running · Agent is working')
    expect(within(screen.getByRole('region', { name: 'Running' })).getByRole('button', { name: 'Deliver search' })).toBeInTheDocument()
})

it('renders the transcript with linked run events and the close notice, and replies post messages', async () => {
    task = mission({ status: 'needs_you', conversation_id: 'task-1', started_at: 't', fields: { ...fields, description: 'Ship search' } })
    snapshot = conversation([
        turn('u1', 'user', 'Begin work on this mission.'),
        turn('a1', 'assistant', 'Launched the build.'),
        turn('u2', 'user', 'Run run-build (work/build.yaml, "Build") ended completed.\n\nUser: Prefer small diffs'),
        turn('n1', 'system', 'Closed as done: Shipped', 'mission_notice'),
    ])
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(await detail().findByText('Launched the build.')).toBeInTheDocument()
    expect(detail().getByText('Closed as done: Shipped')).toBeInTheDocument()
    const events = within(detail().getByLabelText('Mission events'))
    expect(events.getByText('User: Prefer small diffs')).toBeInTheDocument()
    fireEvent.change(detail().getByLabelText('Reply'), { target: { value: 'Ship it' } })
    fireEvent.click(detail().getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(calls.at(-1)).toEqual({ url: '/workspace/api/missions/task-1/events?project_path=%2Fproject', body: { kind: 'human.message', payload: { message: 'Ship it' } } }))
    await waitFor(() => expect(detail().getByLabelText('Reply')).toHaveValue(''))
    fireEvent.click(events.getByRole('button', { name: 'Open run run-build' }))
    expect(useStore.getState().viewMode).toBe('runs')
})

it('drives cancel, close, archive, and budget from the header menu', async () => {
    task = mission({ status: 'running', conversation_id: 'task-1', started_at: 't', fields: { ...fields, budget: { concurrent_runs: 4, total_runs: 25 } } })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    menu('Budget')
    const budget = within(detail().getByRole('form', { name: 'Budget' }))
    expect(budget.getByLabelText('Total runs')).toHaveValue(25)
    fireEvent.change(budget.getByLabelText('Total runs'), { target: { value: '40' } })
    fireEvent.click(budget.getByRole('button', { name: 'Save budget' }))
    await waitFor(() => expect(detail().queryByRole('form', { name: 'Budget' })).not.toBeInTheDocument())
    expect(calls.at(-1)?.body).toEqual({ revision: 1, fields: { budget: { concurrent_runs: 4, total_runs: 40 } }, actor: 'human' })
    menu('Close mission')
    await waitFor(() => expect(calls.at(-1)).toEqual({ url: '/workspace/api/missions/task-1/close?project_path=%2Fproject', body: { status: 'done' } }))
    menu('Cancel mission')
    await waitFor(() => expect(calls.at(-1)?.url).toBe('/workspace/api/missions/task-1/cancel?project_path=%2Fproject'))
    menu('Archive')
    await waitFor(() => expect(task.fields.archived).toBe(true))
    await waitFor(() => expect(detail().getByText('Archived')).toBeInTheDocument())
})

it('sets a draft mission\'s model on its conversation before Start', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    menu('Model')
    const form = within(await detail().findByRole('form', { name: 'Model' }))
    fireEvent.click(form.getByRole('button', { name: /^Model:/ }))
    fireEvent.click(await screen.findByRole('option', { name: 'Opus' }))
    fireEvent.click(screen.getByRole('button', { name: 'High', exact: true }))
    fireEvent.click(form.getByRole('button', { name: 'Save model' }))
    await waitFor(() => expect(detail().queryByRole('form', { name: 'Model' })).not.toBeInTheDocument())
    expect(calls.at(-1)).toEqual({ url: '/workspace/api/conversations/task-1/settings', body: { project_path: '/project', expected_revision: '0', model_settings: { provider: 'claude-code', llm_profile: null, model: 'opus', reasoning_effort: 'high' } } })
})

it('starts a clean pane with its own transcript when another mission is selected', async () => {
    const a = mission({ id: 'a', status: 'running', conversation_id: 'conv-a', started_at: 't', fields: { ...fields, title: 'Mission A', budget: { concurrent_runs: 2, total_runs: 9 } } })
    const b = mission({ id: 'b', status: 'running', conversation_id: 'conv-b', started_at: 't', fields: { ...fields, title: 'Mission B' } })
    const snapshots: Record<string, unknown> = { 'conv-a': conversation([turn('a1', 'assistant', 'Working on A.')]), 'conv-b': conversation([turn('b1', 'assistant', 'Working on B.')]) }
    let releaseB = () => {}
    vi.mocked(fetch).mockImplementation(async url => {
        const id = /conversations\/([^/?]+)/.exec(String(url))?.[1]
        if (!id) return { ok: true, json: async () => ({ missions: [a, b] }) } as Response
        if (id === 'conv-b') await new Promise<void>(resolve => { releaseB = resolve })
        return { ok: true, json: async () => snapshots[id] } as Response
    })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Mission A' }))
    expect(await detail().findByText('Working on A.')).toBeInTheDocument()
    menu('Budget')
    fireEvent.change(detail().getByLabelText('Reply'), { target: { value: 'For A only' } })
    fireEvent.click(screen.getByRole('button', { name: 'Mission B' }))
    await waitFor(() => expect(detail().getByRole('heading', { name: 'Mission B' })).toBeInTheDocument())
    expect(detail().queryByRole('form', { name: 'Budget' })).not.toBeInTheDocument()
    expect(detail().getByLabelText('Reply')).toHaveValue('')
    expect(detail().queryByText('Working on A.')).not.toBeInTheDocument()
    await act(async () => { releaseB() })
    expect(await detail().findByText('Working on B.')).toBeInTheDocument()
})

it('shows the server reason when a control action is refused', async () => {
    task = mission({ status: 'running', conversation_id: 'task-1', started_at: 't' })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    vi.mocked(fetch).mockResolvedValueOnce({ ok: false, status: 409, json: async () => ({ detail: 'Mission is closed' }) } as Response)
    menu('Close mission')
    expect(await detail().findByText('Mission is closed')).toBeInTheDocument()
})

it('renders inline flow launches in the transcript with a link to the run', async () => {
    task = mission({ status: 'running', conversation_id: 'task-1', started_at: 't' })
    const at = '2026-09-20T10:00:00Z'
    snapshot = { ...conversation([turn('a1', 'assistant', 'Launching the build.')]),
        segments: [{ id: 'seg-launch', turn_id: 'a1', order: 1, kind: 'flow_launch', role: 'system', status: 'complete', timestamp: at, updated_at: at, revision: 1, completed_at: at, content: '', artifact_id: 'launch-1', error: null, tool_call: null, source: null }],
        flow_launches: [{ id: 'launch-1', created_at: at, updated_at: at, revision: 1, flow_name: 'work/build.yaml', summary: 'Build the search index', project_path: '/project', conversation_id: 'task-1', source_turn_id: 'a1', source_segment_id: 'seg-launch', status: 'launched', goal: null, launch_context: null, model: null, run_id: 'run-launch', launch_error: null }] }
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(await detail().findByText('Build the search index')).toBeInTheDocument()
    fireEvent.click(detail().getByRole('button', { name: 'Open run' }))
    expect(useStore.getState().viewMode).toBe('runs')
})

it('disables cancel and close for a closed mission and hides the reply box', async () => {
    task = mission({ status: 'closed', conversation_id: 'task-1', started_at: 't', closed: { status: 'canceled', reason: 'Canceled by human', at: 't' } })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(detail().queryByLabelText('Reply')).not.toBeInTheDocument()
    expect(detail().getByText('This mission is closed.')).toBeInTheDocument()
    fireEvent.keyDown(detail().getByRole('button', { name: 'Mission actions' }), { key: 'Enter' })
    expect(screen.getByRole('menuitem', { name: 'Cancel mission' })).toHaveAttribute('aria-disabled', 'true')
    expect(screen.getByRole('menuitem', { name: 'Close mission' })).toHaveAttribute('aria-disabled', 'true')
})

it('preserves drafts across refresh and tab activation, and reconciles concurrent changes', async () => {
    const view = render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'My draft' } })
    task = { ...task, revision: 2, fields: { ...task.fields, description: 'Server objective' } }
    fireEvent.click(screen.getByRole('button', { name: /^Save/ }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Your edits are preserved')
    view.rerender(<MissionsPanel active={false} />)
    view.rerender(<MissionsPanel active />)
    expect(await screen.findByRole('status')).toHaveTextContent('newer revision')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Reconcile with latest revision' }))
    expect(screen.getByLabelText('Objective')).toHaveValue('Server objective')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    fireEvent.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(task.revision).toBe(3))
    expect(task.fields.description).toBe('Server objective')
})

it('loads on activation without polling and preserves drafts when opening other missions', async () => {
    const view = render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unfinished edit' } })
    fireEvent.change(screen.getByLabelText('Objective'), { target: { value: 'Unfinished note' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Unfinished edit')
    expect(screen.getByLabelText('Objective')).toHaveValue('Unfinished note')
    view.rerender(<MissionsPanel active={false} />)
    vi.useFakeTimers()
    const count = vi.mocked(fetch).mock.calls.length
    view.rerender(<MissionsPanel active />)
    expect(vi.mocked(fetch).mock.calls.length).toBe(count + 1)
    await act(async () => { vi.advanceTimersByTime(60000) })
    expect(vi.mocked(fetch).mock.calls.length).toBe(count + 1)
})

it('keeps unsaved edits scoped to their project when switching projects', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A draft' } })
    fireEvent.change(screen.getByLabelText('Objective'), { target: { value: 'Project A note' } })
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    await act(async () => { useStore.setState({ activeProjectPath: '/project' }) })
    expect(screen.getByLabelText('Title')).toHaveValue('Project A draft')
    expect(screen.getByLabelText('Objective')).toHaveValue('Project A note')
})

it('creates with only a title, keeps the saved mission open and resets its baseline', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    expect(screen.getByLabelText('Title')).toHaveFocus()
    expect(screen.getByLabelText('Title')).toHaveAttribute('data-slot', 'input')
    expect(screen.getByLabelText('Objective')).toHaveAttribute('data-slot', 'textarea')
    expect(screen.queryByLabelText('Stage')).not.toBeInTheDocument()
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    expect(screen.queryByText('Activity')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Quick capture' } })
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.click(editor().getByRole('button', { name: 'Create mission' }))
    await waitFor(() => expect(editor().getByRole('heading', { name: 'Quick capture' })).toHaveFocus())
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    expect(task.fields.title).toBe('Quick capture')
    expect(calls).toHaveLength(1)
    expect(calls[0].body.revision).toBeUndefined()
})

it('creates with a picked playbook and shows it under the objective with its text expandable', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Fix parser' } })
    expect(screen.getByLabelText('Playbook')).toHaveValue('')
    await screen.findByRole('option', { name: 'Bug report' })
    fireEvent.change(screen.getByLabelText('Playbook'), { target: { value: 'bug-report' } })
    fireEvent.click(editor().getByRole('button', { name: 'Create mission' }))
    await waitFor(() => expect(calls).toHaveLength(1))
    expect(calls[0].body.fields).toMatchObject({ title: 'Fix parser', playbook: 'bug-report' })
    await waitFor(() => expect(within(detail().getByRole('region', { name: 'Objective' })).getByText('bug-report')).toBeInTheDocument())

    task = mission({ status: 'running', conversation_id: 'task-new', started_at: 't', playbook: { name: 'bug-report', title: 'Bug report', description: 'Fix a bug.', text: 'Reproduce first.' } })
    act(() => { window.dispatchEvent(new CustomEvent('spark:mission-live-event', { detail: { projectPath: '/project', mission: task } })) })
    const objective = within(detail().getByRole('region', { name: 'Objective' }))
    const text = await objective.findByText('Reproduce first.')
    expect(text).not.toBeVisible()
    fireEvent.click(objective.getByText('bug-report'))
    expect(text).toBeVisible()
})

it('preserves a new draft on Cancel', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Draft title' } })
    fireEvent.click(editor().getByRole('button', { name: 'Cancel' }))
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    expect(screen.getByLabelText('Title')).toHaveValue('Draft title')
})

it('tracks reverted fields and discards to the latest revision', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    expect(screen.getByLabelText('Title')).toHaveFocus()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Changed' } })
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Deliver search' } })
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Objective'), { target: { value: 'Note only' } })
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Objective')).toHaveValue('Note only')
    task = { ...task, revision: 2, fields: { ...task.fields, title: 'Server title' } }
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await screen.findByRole('status')
    fireEvent.click(editor().getByRole('button', { name: /^Discard/ }))
    expect(editor().getByRole('heading', { name: 'Server title' })).toHaveFocus()
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
})

it('locks editing and dismissal during saving and preserves input after failure', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Keep this' } })
    let reject!: (error: Error) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail }))
    fireEvent.click(editor().getByRole('button', { name: /^Save/ }))
    expect(screen.getByLabelText('Title')).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Close' })).toBeDisabled()
    fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
    expect(screen.getByLabelText('Title')).toBeInTheDocument()
    await act(async () => { reject(new Error('Save unavailable')) })
    expect(await screen.findByRole('alert')).toHaveTextContent('Save unavailable')
    expect(screen.getByLabelText('Title')).toHaveValue('Keep this')
})

it('replaces the list at the narrow breakpoint and dismisses on Escape', async () => {
    window.innerWidth = 1024
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    expect(screen.queryByRole('region', { name: 'Drafts' })).not.toBeInTheDocument()
    fireEvent.keyDown(detail().getByRole('heading', { level: 2 }), { key: 'Escape' })
    expect(screen.getByRole('region', { name: 'Drafts' })).toBeInTheDocument()
    // jsdom lays nothing out, so focus falls back to the list heading.
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Missions' })).toHaveFocus())
})

it('keeps conflicts blocked after dismissal when the latest revision cannot be loaded', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Conflicting edit' } })
    vi.mocked(fetch).mockResolvedValueOnce({ ok: false, status: 409, json: async () => ({}) } as Response)
    vi.mocked(fetch).mockRejectedValueOnce(new Error('Refresh unavailable'))
    fireEvent.click(editor().getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(editor().getByRole('button', { name: 'Close' })).toBeEnabled())
    fireEvent.click(editor().getByRole('button', { name: 'Close' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Conflicting edit')
    expect(screen.getByRole('status')).toHaveTextContent('reconcile before saving')
    expect(editor().getByRole('button', { name: /^Save/ })).toBeDisabled()
    fireEvent.click(editor().getByRole('button', { name: /^Discard/ }))
    expect(detail().getByRole('button', { name: 'Mission actions' })).toBeEnabled()
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
})

const live = (projectPath: string, update: unknown) => act(async () => {
    window.dispatchEvent(new CustomEvent('spark:mission-live-event', { detail: { projectPath, mission: update } }))
})
it('loads only the selected project and applies live upserts without polling', async () => {
    const view = render(<MissionsPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    vi.useFakeTimers()
    vi.mocked(fetch).mockClear()
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    await act(async () => { vi.advanceTimersByTime(30000) })
    expect(vi.mocked(fetch).mock.calls.map(([url]) => url)).toEqual(['/workspace/api/missions?project_path=%2Fother'])
    await live('/project', { ...task, id: 'task-elsewhere', fields: { ...task.fields, title: 'Wrong project' } })
    expect(screen.queryByRole('button', { name: 'Wrong project' })).not.toBeInTheDocument()
    await live('/other', { ...task, id: 'task-live', status: 'needs_you', fields: { ...task.fields, title: 'Arrived live' } })
    expect(within(screen.getByRole('region', { name: 'Needs you' })).getByRole('button', { name: 'Arrived live' })).toBeInTheDocument()
    vi.mocked(fetch).mockClear()
    await live('/other', null)
    expect(vi.mocked(fetch).mock.calls.map(([url]) => url)).toEqual(['/workspace/api/missions?project_path=%2Fother'])
    view.rerender(<MissionsPanel active={false} />)
    vi.mocked(fetch).mockClear()
    await live('/other', null)
    await act(async () => { vi.advanceTimersByTime(30000) })
    expect(fetch).not.toHaveBeenCalled()
})

it('archives from the editor separately without saving or losing edited fields', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unsaved title' } })
    fireEvent.click(screen.getByRole('button', { name: 'Archive mission' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore mission' })).toBeEnabled())
    expect(calls[0].body.fields).toEqual({ archived: true })
    expect(task.fields.title).toBe('Deliver search')
    expect(screen.getByLabelText('Title')).toHaveValue('Unsaved title')
    fireEvent.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(task.fields.title).toBe('Unsaved title'))
})

it('filters titles locally with matching counts and keeps the selected read view', async () => {
    task.fields.description = 'Paragraph one.\n\nParagraph two.'
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(detail().getByRole('heading', { level: 2 })).toHaveFocus()
    expect(detail().getByText(task.fields.description, { normalizer: value => value })).toHaveClass('whitespace-pre-wrap')
    const count = vi.mocked(fetch).mock.calls.length
    fireEvent.change(screen.getByLabelText('Search titles'), { target: { value: 'DELIVER' } })
    expect(within(screen.getByRole('region', { name: 'Drafts' })).getByLabelText('1 matching missions')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Search titles'), { target: { value: 'missing' } })
    expect(screen.queryByRole('button', { name: 'Deliver search' })).not.toBeInTheDocument()
    expect(screen.getByText('No matches')).toBeInTheDocument()
    expect(detail().getByRole('heading', { level: 2 })).toHaveTextContent('Deliver search')
    fireEvent.click(screen.getByRole('button', { name: 'Clear search' }))
    expect(screen.getByRole('button', { name: 'Deliver search' })).toHaveAttribute('aria-pressed', 'true')
    expect(fetch).toHaveBeenCalledTimes(count)
})
