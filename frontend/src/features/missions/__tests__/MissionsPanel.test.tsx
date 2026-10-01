import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TriggersSessionController } from '@/app/AppSessionControllers'
import { TriggersPanel } from '@/features/triggers/TriggersPanel'
import { createEmptyTriggerForm } from '@/features/triggers/model/triggerForm'
import { MissionsPanel } from '../MissionsPanel'
import { useStore } from '@/store'

const fields = { title: 'Deliver search', description: '', archived: false }
type Task = { id: string; revision: number; fields: typeof fields; activity: unknown[]; status?: string; updated_at?: string; [key: string]: unknown }
let task: Task
let calls: { url: string; body: Record<string, unknown> }[]
let projectUses: { project_path: string; last_accessed_at?: string }[]
let snapshot: Record<string, unknown>
beforeEach(() => {
    window.innerWidth = 1440
    task = { id: 'task-1', project_path: '/project', revision: 1, fields: { ...fields }, activity: [], status: 'draft', updated_at: '2026-09-20 10:00:00.0 +00:00:00' }
    calls = []
    projectUses = []
    snapshot = conversation([])
    useStore.setState({ activeProjectPath: '/project', viewMode: 'missions', missionBoard: [], selectedMission: null, projectRegistry: {
        '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-09-20T10:00:00Z' },
        '/other': { directoryPath: '/other', isFavorite: false, lastAccessedAt: null },
    } })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        if (url === '/workspace/api/triggers') return { ok: true, json: async () => [] }
        if (url.includes('/chat-models')) return { ok: true, json: async () => ({ models: [{ provider: 'claude-code', id: 'opus', display: 'Opus', is_default: true, supported_reasoning_efforts: ['high'] }], providers: { codex: { status: 'unavailable', error: null } } }) }
        if (url.endsWith('/projects/state')) {
            const body = JSON.parse(String(init!.body)); projectUses.push(body)
            return { ok: true, status: 200, json: async () => ({ project_id: body.project_path, project_path: body.project_path, display_name: body.project_path.slice(1), created_at: '2026-09-01T00:00:00Z', last_opened_at: null, last_accessed_at: body.last_accessed_at, is_favorite: false, active_conversation_id: null }) }
        }
        if (init?.body) {
            const body = JSON.parse(String(init.body)); calls.push({ url, body })
            if (url.includes('/conversations/')) return { ok: true, json: async () => snapshot }
            if (init.method === 'PATCH' && body.revision !== task.revision) return { ok: false, status: 409, json: async () => ({ detail: 'Conflict' }) }
            if (init.method === 'POST' && !url.includes('/missions/')) task = { ...task, id: 'task-new', revision: 1, fields: { ...task.fields, ...body.fields } }
            else if (body.fields) task = { ...task, revision: task.revision + 1, fields: { ...task.fields, ...body.fields } }
            return { ok: true, json: async () => task }
        }
        if (url.includes('/conversations/')) return { ok: true, json: async () => snapshot }
        if (url === '/workspace/api/playbooks/bug-report') return { ok: true, json: async () => ({ name: 'bug-report', title: 'Bug report', description: 'Fix a bug.', text: 'Reproduce the draft first.' }) }
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
/** Starts a new mission, picking its project. */
async function newMission(project = '/project') {
    fireEvent.keyDown(screen.getByRole('button', { name: 'New mission' }), { key: 'Enter' })
    fireEvent.click(screen.getAllByTestId('project-picker-item').find(item => item.dataset.projectPath === project)!)
    await screen.findByLabelText('Title')
}
const mission = (overrides: Partial<Task>) => ({ ...task, ...overrides, fields: { ...task.fields, ...(overrides.fields ?? {}) } })

it('groups missions by derived status, newest update first, hiding empty groups', async () => {
    const board = [
        mission({ id: 'a', status: 'running', updated_at: '2026-09-20 09:00:00.0 +00:00:00', fields: { ...fields, title: 'Older running' }, runs: [{ run_id: 'r1', flow_name: 'work/build.yaml', summary: 'Build', launched_at: 't', status: 'running' }] }),
        mission({ id: 'b', status: 'running', updated_at: '2026-09-21 09:00:00.0 +00:00:00', fields: { ...fields, title: 'Newer running' } }),
        mission({ id: 'c', status: 'needs_you', fields: { ...fields, title: 'Gated' }, runs: [{ run_id: 'r2', flow_name: 'work/review.yaml', summary: 'Review it', launched_at: 't', status: 'waiting' }], question: { prompt: 'Ship the review?', options: [{ label: 'Ship it', value: 'ship' }], root_run_id: 'r2' } }),
        mission({ id: 'd', status: 'closed', fields: { ...fields, title: 'Finished' }, closed: { status: 'done', reason: 'Shipped', at: '2026-09-20 10:00:00.0 +00:00:00' } }),
    ]
    vi.mocked(fetch).mockImplementation(async () => ({ ok: true, json: async () => ({ missions: board }) }) as Response)
    render(<MissionsPanel active />)
    await screen.findByRole('button', { name: 'Gated' })
    expect(screen.getAllByRole('heading', { level: 3 }).map(heading => heading.firstChild?.textContent)).toEqual(['Needs you', 'Running', 'Closed'])
    const running = within(screen.getByRole('region', { name: 'Running' }))
    expect(running.getAllByRole('button').map(button => button.getAttribute('aria-label'))).toEqual(['Newer running', 'Older running'])
    expect(running.getByText(/^Build · /)).toBeInTheDocument()
    expect(running.getByText('Agent is working')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Gated' })).toHaveAccessibleDescription('Ship the review?')
    // A closed row says how and when it closed, never why.
    expect(screen.getByRole('button', { name: 'Finished' }).getAttribute('aria-describedby')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Finished' })).toHaveAccessibleDescription(/^Done · Sep 20/)
    expect(screen.queryByText(/Shipped/)).not.toBeInTheDocument()
    expect(screen.queryByRole('region', { name: 'Drafts' })).not.toBeInTheDocument()
    // Archive and Restore live in the mission's actions, not on the rows.
    expect(screen.queryAllByRole('button', { name: /^(Archive|Restore)/ })).toEqual([])
})

it('archives closed rows and shows them again with Show archived', async () => {
    task = mission({ status: 'closed', closed: { status: 'done', reason: 'Shipped', at: 't' } })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.click(detail().getByRole('button', { name: 'Archive' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Deliver search' })).not.toBeInTheDocument())
    expect(calls[0].body).toEqual({ revision: 1, fields: { archived: true }, actor: 'human' })
    fireEvent.click(screen.getByRole('button', { name: 'Show archived' }))
    expect(screen.getByRole('button', { name: 'Deliver search' })).toHaveAccessibleDescription(/ · Archived$/)
    fireEvent.click(detail().getByRole('button', { name: 'Restore' }))
    await waitFor(() => expect(task.fields.archived).toBe(false))
    fireEvent.click(screen.getByRole('button', { name: 'Hide archived' }))
    expect(screen.getByRole('button', { name: 'Deliver search' })).toBeInTheDocument()
})

it('shows a draft with its objective in the rail, Start and Edit, and the reply box disabled', async () => {
    task.fields.description = 'Search should return matching documents.'
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(within(detail().getByRole('complementary', { name: 'Mission overview' })).getByRole('region', { name: 'Objective' })).toHaveTextContent(task.fields.description)
    expect(detail().getByText(/No conversation yet/)).toBeInTheDocument()
    expect(detail().getByLabelText('Reply')).toBeDisabled()
    expect(detail().getByRole('button', { name: 'Edit' })).toBeInTheDocument()
    expect(detail().queryByRole('button', { name: 'Close mission' })).not.toBeInTheDocument()
    vi.mocked(fetch).mockImplementationOnce(async (url, init) => {
        calls.push({ url: String(url), body: JSON.parse(String(init?.body)) })
        task = mission({ status: 'running', conversation_id: 'task-1', started_at: 't' })
        return { ok: true, json: async () => task } as Response
    })
    fireEvent.click(detail().getByRole('button', { name: 'Start' }))
    await waitFor(() => expect(detail().getByLabelText('Reply')).toBeEnabled())
    expect(calls[0].url).toBe('/workspace/api/missions/task-1/start?project_path=%2Fproject')
    expect(detail().getByRole('status')).toHaveTextContent('Running · Agent is working')
    expect(within(screen.getByRole('region', { name: 'Running' })).getByRole('button', { name: 'Deliver search' })).toBeInTheDocument()
})

it('reads the thread as quiet event lines, ends with the pending question, and replies post messages', async () => {
    const question = { flow_name: 'work/review.yaml', options: [{ key: 'S', label: 'Ship it', value: 'Ship it' }, { key: 'H', label: 'Hold', value: 'Hold' }], prompt: 'Ship the build?', question_id: 'q-1', root_run_id: 'run-build', run_id: 'run-build' }
    task = mission({ status: 'needs_you', conversation_id: 'task-1', started_at: 't', fields: { ...fields, description: 'Ship search' }, question,
        runs: [{ run_id: 'run-build', flow_name: 'work/build.yaml', summary: 'Build', launched_at: 't', status: 'waiting' }] })
    snapshot = conversation([
        turn('u1', 'user', 'Begin work on this mission.'),
        turn('a1', 'assistant', 'Launched the build.'),
        turn('u2', 'user', 'Run run-build (work/build.yaml, "Build") ended completed.\n\nUser: Prefer small diffs'),
        turn('u3', 'user', `Run question: ${JSON.stringify(question)}`),
        turn('a2', 'assistant', 'Should I ship the build?'),
    ])
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(await detail().findByText('Launched the build.')).toBeInTheDocument()
    // The objective shows once, in the rail; the thread opens with a quiet start line.
    expect(detail().getAllByText('Ship search')).toHaveLength(1)
    const events = detail().getAllByTestId('mission-event')
    expect(events.map(event => [event.firstChild?.textContent, event.children[1].textContent])).toEqual([['▸', 'Mission started'], ['✓', 'Build ended completed'], ['?', 'Build asked “Ship the build?”']])
    expect(detail().getByText('Prefer small diffs')).toBeInTheDocument()
    const ask = within(detail().getByRole('region', { name: 'Waiting on you' }))
    expect(ask.getByText('Ship the build?')).toBeInTheDocument()
    fireEvent.click(ask.getByRole('button', { name: 'Ship it' }))
    await waitFor(() => expect(calls.at(-1)).toEqual({ url: '/workspace/api/missions/task-1/events?project_path=%2Fproject', body: { kind: 'human.message', payload: { message: 'Ship it' } } }))
    fireEvent.change(detail().getByLabelText('Reply'), { target: { value: 'Hold for now' } })
    fireEvent.click(detail().getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(calls.at(-1)?.body).toEqual({ kind: 'human.message', payload: { message: 'Hold for now' } }))
    await waitFor(() => expect(detail().getByLabelText('Reply')).toHaveValue(''))
    // The rail lists the run with a link to it on the Runs page.
    const runs = within(detail().getByRole('region', { name: 'Runs' }))
    expect(runs.getByRole('img', { name: 'Waiting' })).toBeInTheDocument()
    expect(runs.getByRole('link', { name: 'Build' })).toHaveAttribute('href', '#/runs/run-build')
    fireEvent.click(within(events[1]).getByRole('link', { name: 'Build' }))
    expect(useStore.getState().viewMode).toBe('runs')
})

it('opens with the start line when the first turn repeats the objective', async () => {
    task = mission({ status: 'running', conversation_id: 'task-1', started_at: 't', fields: { ...fields, description: 'Ship search' } })
    snapshot = conversation([
        turn('u1', 'user', 'Objective:\nShip search\n\nBegin work on this mission.'),
        turn('a1', 'assistant', 'On it.'),
    ])
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(await detail().findByText('On it.')).toBeInTheDocument()
    expect(detail().getAllByText('Ship search')).toHaveLength(1)
    expect(detail().getAllByTestId('mission-event').map(event => event.children[1].textContent)).toEqual(['Mission started'])
})

it('drives close, cancel and archive from the header, and budget in the rail', async () => {
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
    expect(detail().getByRole('region', { name: 'Budget' })).toHaveTextContent('0 of 40 runs · up to 4 at once')
    fireEvent.click(detail().getByRole('button', { name: 'Close mission' }))
    await waitFor(() => expect(calls.at(-1)).toEqual({ url: '/workspace/api/missions/task-1/close?project_path=%2Fproject', body: { status: 'done' } }))
    fireEvent.click(detail().getByRole('button', { name: 'Cancel mission' }))
    await waitFor(() => expect(calls.at(-1)?.url).toBe('/workspace/api/missions/task-1/cancel?project_path=%2Fproject'))
    menu('Archive')
    await waitFor(() => expect(task.fields.archived).toBe(true))
    await waitFor(() => expect(detail().getByRole('status')).toHaveTextContent('· Archived'))
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
    fireEvent.click(detail().getByRole('button', { name: 'Close mission' }))
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
    const launch = (await detail().findAllByTestId('mission-event'))[0]
    expect(launch).toHaveTextContent(/^→Launched Build the search index/)
    fireEvent.click(within(launch).getByRole('link', { name: 'Build the search index' }))
    expect(useStore.getState().viewMode).toBe('runs')
})

it('answers a question the agent asks through its own tool mid-turn', async () => {
    task = mission({ status: 'running', conversation_id: 'task-1', started_at: 't' })
    const at = '2026-09-20T10:00:00Z'
    snapshot = { ...conversation([{ ...turn('a1', 'assistant', ''), status: 'streaming' }]),
        segments: [{ id: 'seg-ask', turn_id: 'a1', order: 1, kind: 'request_user_input', role: 'system', status: 'pending', timestamp: at, updated_at: at, completed_at: null, content: '', error: null, tool_call: null, source: null,
            request_user_input: { request_id: 'req-1', status: 'pending', questions: [{ id: 'q1', header: 'Publish?', question: 'Publish the report?', question_type: 'MULTIPLE_CHOICE', options: [{ label: 'Publish', description: null }, { label: 'Hold', description: null }], allow_other: true, is_secret: false }], answers: {}, submitted_at: null } }] }
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.click(await detail().findByRole('button', { name: 'Publish', exact: true }))
    fireEvent.click(detail().getByRole('button', { name: 'Submit' }))
    await waitFor(() => expect(calls.at(-1)).toEqual({ url: '/workspace/api/conversations/task-1/request-user-input/req-1/answer', body: { project_path: '/project', answers: { q1: 'Publish' } } }))
})

it('ends a closed mission with its outcome once, without cancel, close or the reply box', async () => {
    task = mission({ status: 'closed', conversation_id: 'task-1', started_at: 't', closed: { status: 'canceled', reason: 'Canceled by human', at: '2026-09-20 10:00:00.0 +00:00:00' } })
    snapshot = conversation([turn('a1', 'assistant', 'Stopping.'), turn('n1', 'system', 'Closed as canceled: Canceled by human', 'mission_notice')])
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(await detail().findByText('Stopping.')).toBeInTheDocument()
    expect(detail().queryByLabelText('Reply')).not.toBeInTheDocument()
    expect(detail().getByRole('region', { name: 'Outcome' })).toHaveTextContent(/^Closed as canceled · Sep 20.*Canceled by human$/)
    expect(detail().getAllByText(/Canceled by human/)).toHaveLength(1)
    expect(detail().getByRole('status')).toHaveTextContent(/^Canceled · Sep 20/)
    expect(detail().queryByRole('button', { name: 'Close mission' })).not.toBeInTheDocument()
    expect(detail().queryByRole('button', { name: 'Cancel mission' })).not.toBeInTheDocument()
    fireEvent.keyDown(detail().getByRole('button', { name: 'Mission actions' }), { key: 'Enter' })
    expect(screen.getAllByRole('menuitem').map(item => item.textContent)).toEqual(['Edit', 'Model', 'Budget', 'Archive'])
    expect(screen.getByRole('menuitem', { name: 'Model' })).toHaveAttribute('aria-disabled', 'true')
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

it('loads once without polling and preserves drafts when opening other missions', async () => {
    const view = render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unfinished edit' } })
    fireEvent.change(screen.getByLabelText('Objective'), { target: { value: 'Unfinished note' } })
    await newMission()
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Unfinished edit')
    expect(screen.getByLabelText('Objective')).toHaveValue('Unfinished note')
    view.rerender(<MissionsPanel active={false} />)
    vi.useFakeTimers()
    const count = vi.mocked(fetch).mock.calls.length
    view.rerender(<MissionsPanel active />)
    await act(async () => { vi.advanceTimersByTime(60000) })
    expect(vi.mocked(fetch).mock.calls.length).toBe(count)
})

it('switches to a mission selected elsewhere while editing or drafting, keeping the draft', async () => {
    const other = mission({ id: 'task-2', fields: { ...fields, title: 'Other mission' } })
    vi.mocked(fetch).mockImplementation(async () => ({ ok: true, json: async () => ({ missions: [task, other] }) }) as Response)
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unfinished edit' } })
    await act(async () => { useStore.getState().setSelectedMission({ id: 'task-2', projectPath: '/project' }) })
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Other mission' })).toHaveAttribute('aria-pressed', 'true')
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Unfinished edit')

    await newMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'New draft' } })
    await act(async () => { useStore.getState().setSelectedMission({ id: 'task-2', projectPath: '/project' }) })
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Other mission' })).toHaveAttribute('aria-pressed', 'true')
    await newMission()
    expect(screen.getByLabelText('Title')).toHaveValue('New draft')
})

it('keeps unsaved edits while another view is open', async () => {
    const view = render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    editMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A draft' } })
    fireEvent.change(screen.getByLabelText('Objective'), { target: { value: 'Project A note' } })
    await act(async () => { useStore.setState({ activeProjectPath: '/other', viewMode: 'runs' }) })
    view.rerender(<MissionsPanel active={false} />)
    view.rerender(<MissionsPanel active />)
    expect(screen.getByLabelText('Title')).toHaveValue('Project A draft')
    expect(screen.getByLabelText('Objective')).toHaveValue('Project A note')
})

it('lists every project\'s missions with the project on each row, and asks which project a new one goes to', async () => {
    const board = [mission({ id: 'here', fields: { ...fields, title: 'Here' } }), mission({ id: 'there', project_path: '/other', fields: { ...fields, title: 'There' } })]
    const base = vi.mocked(fetch).getMockImplementation()!
    vi.mocked(fetch).mockImplementation(async (url, init) => {
        if (String(url).endsWith('/projects/state')) return base(url, init)
        if (init?.body) { calls.push({ url: String(url), body: JSON.parse(String(init.body)) }); return { ok: true, json: async () => ({ ...board[1], id: 'created', fields: { ...fields, title: 'New there' } }) } as Response }
        return { ok: true, json: async () => ({ missions: board }) } as Response
    })
    render(<MissionsPanel active />)
    await screen.findByRole('button', { name: 'There' })
    expect(vi.mocked(fetch).mock.calls[0][0]).toBe('/workspace/api/missions')
    expect(screen.getAllByTestId('mission-project').map(row => row.textContent)).toEqual(['project ·', 'other ·'])
    fireEvent.keyDown(screen.getByRole('button', { name: 'New mission' }), { key: 'Enter' })
    // The last-used project is suggested first.
    expect(screen.getAllByTestId('project-picker-item').map(item => item.dataset.projectPath)).toEqual(['/project', '/other'])
    fireEvent.click(screen.getAllByTestId('project-picker-item')[1])
    expect(await screen.findByTestId('new-mission-project')).toHaveTextContent('New mission in other')
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'New there' } })
    fireEvent.click(editor().getByRole('button', { name: 'Create mission' }))
    await waitFor(() => expect(calls.at(-1)?.url).toBe('/workspace/api/missions?project_path=%2Fother'))
    // Creating it records the project as last used, so the next new mission suggests it first.
    await waitFor(() => expect(projectUses).toEqual([expect.objectContaining({ project_path: '/other', last_accessed_at: expect.any(String) })]))
    fireEvent.keyDown(screen.getByRole('button', { name: 'New mission' }), { key: 'Enter' })
    expect(screen.getAllByTestId('project-picker-item').map(item => item.dataset.projectPath)).toEqual(['/other', '/project'])
})

it('creates with only a title, keeps the saved mission open and resets its baseline', async () => {
    render(<MissionsPanel active />)
    await newMission()
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

it('creates with a picked playbook and shows it in the rail with its text expandable', async () => {
    render(<MissionsPanel active />)
    await newMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Fix parser' } })
    expect(screen.getByLabelText('Playbook')).toHaveValue('')
    await screen.findByRole('option', { name: 'Bug report' })
    fireEvent.change(screen.getByLabelText('Playbook'), { target: { value: 'bug-report' } })
    fireEvent.click(editor().getByRole('button', { name: 'Create mission' }))
    await waitFor(() => expect(calls).toHaveLength(1))
    expect(calls[0].body.fields).toMatchObject({ title: 'Fix parser', playbook: 'bug-report' })
    // Before Start the rail loads the named playbook's current text.
    const draftPlaybook = within(detail().getByRole('region', { name: 'Playbook' }))
    const draftText = await draftPlaybook.findByText('Reproduce the draft first.')
    expect(draftText).not.toBeVisible()
    fireEvent.click(draftPlaybook.getByText('bug-report'))
    expect(draftText).toBeVisible()

    task = mission({ status: 'running', conversation_id: 'task-new', started_at: 't', playbook: { name: 'bug-report', title: 'Bug report', description: 'Fix a bug.', text: 'Reproduce first.' } })
    act(() => { window.dispatchEvent(new CustomEvent('spark:mission-live-event', { detail: { projectPath: '/project', mission: task } })) })
    const playbook = within(detail().getByRole('region', { name: 'Playbook' }))
    // After Start the rail shows the frozen text, still expanded.
    const text = await playbook.findByText('Reproduce first.')
    expect(text).toBeVisible()
    fireEvent.click(playbook.getByText('bug-report'))
    expect(text).not.toBeVisible()
})

it('preserves a new draft on Cancel', async () => {
    render(<MissionsPanel active />)
    await newMission()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Draft title' } })
    fireEvent.click(editor().getByRole('button', { name: 'Cancel' }))
    await newMission()
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

it('stacks the list above the mission at the narrow breakpoint and dismisses on Escape', async () => {
    window.innerWidth = 1024
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    expect(screen.getByTestId('missions-view')).toHaveAttribute('data-responsive-layout', 'stacked')
    expect(screen.getByRole('region', { name: 'Drafts' })).toBeInTheDocument()
    fireEvent.keyDown(detail().getByRole('heading', { level: 2 }), { key: 'Escape' })
    expect(screen.queryByRole('region', { name: 'Mission details' })).not.toBeInTheDocument()
    // jsdom lays nothing out, so focus falls back to the list's search.
    await waitFor(() => expect(screen.getByLabelText('Search titles')).toHaveFocus())
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
it('applies live upserts from every project without polling', async () => {
    render(<MissionsPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    vi.useFakeTimers()
    vi.mocked(fetch).mockClear()
    await act(async () => { vi.advanceTimersByTime(30000) })
    expect(fetch).not.toHaveBeenCalled()
    await live('/other', { ...task, id: 'task-live', project_path: '/other', status: 'needs_you', fields: { ...task.fields, title: 'Arrived live' } })
    expect(within(screen.getByRole('region', { name: 'Needs you' })).getByRole('button', { name: 'Arrived live' })).toBeInTheDocument()
    await live('/other', null)
    expect(vi.mocked(fetch).mock.calls.map(([url]) => url)).toEqual(['/workspace/api/missions'])
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

it('shows waiting in Running and links targeting triggers from mission detail', async () => {
    task = { ...task, status: 'running', waiting: true, wait_reason: 'Review pending' }
    const original = vi.mocked(fetch).getMockImplementation()!
    vi.mocked(fetch).mockImplementation(async (url, init) => {
        if (String(url) === '/workspace/api/triggers') return { ok: true, json: async () => [{ id: 'watcher', revision: '1', name: 'Review watcher', source_type: 'webhook', enabled: true, created_at: '', updated_at: '', action: { mode: 'mission', mission_id: task.id, project_path: '/project' }, source: {}, state: {} }] } as Response
        return original(url, init)
    })
    render(<MissionsPanel active />)
    const running = await screen.findByRole('region', { name: 'Running' })
    expect(within(running).getByText('Waiting: Review pending')).toBeVisible()
    expect(screen.queryByRole('region', { name: 'Needs you' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: task.fields.title }))
    fireEvent.click(await screen.findByRole('button', { name: 'Review watcher · webhook · Enabled' }))
    expect(useStore.getState().viewMode).toBe('triggers')
    expect(useStore.getState().triggersSession.selectedTriggerId).toBe('watcher')
})


it.each(['Close mission', 'Cancel mission', 'agent close'])('%s updates mission detail and the cached Triggers view from live events', async (action) => {
    task = mission({ status: 'needs_you', started_at: 't' })
    let trigger = { id: 'watcher', revision: '1', name: 'Review watcher', source_type: 'schedule', enabled: true, created_at: '', updated_at: '', action: { mode: 'mission', mission_id: task.id, project_path: '/project' }, source: { kind: 'interval', interval_seconds: 60 }, state: { recent_history: [], next_run_at: null } }
    useStore.setState({ viewMode: 'triggers', triggersSession: {
        status: 'idle', error: null, triggers: [], selectedTriggerId: null, revealedWebhookSecrets: {}, createFormOpen: false,
        newTriggerDraft: { form: createEmptyTriggerForm(null) }, editTriggerDraftsByTriggerId: {},
    } })
    const original = vi.mocked(fetch).getMockImplementation()!
    const close = () => {
        task = mission({ status: 'closed', closed: { status: action === 'Cancel mission' ? 'canceled' : 'done', actor: action === 'agent close' ? 'assistant' : 'human', reason: 'Finished', at: 't' } })
        trigger = { ...trigger, enabled: false, revision: '2' }
        window.dispatchEvent(new CustomEvent('spark:trigger-live-event', { detail: { type: 'trigger.upsert', payload: { type: 'trigger_upsert', trigger } } }))
        window.dispatchEvent(new CustomEvent('spark:mission-live-event', { detail: { projectPath: '/project', mission: task } }))
    }
    vi.mocked(fetch).mockImplementation(async (url, init) => {
        if (String(url) === '/workspace/api/triggers') return { ok: true, json: async () => [trigger] } as Response
        if (init?.method === 'POST' && /\/(close|cancel)\?/.test(String(url))) {
            close()
            return { ok: true, json: async () => task } as Response
        }
        return original(url, init)
    })
    // Keep both views mounted while switching, as the app does for cached panels.
    function Panels() {
        const view = useStore(state => state.viewMode)
        return <><TriggersSessionController /><div hidden={view !== 'triggers'}><TriggersPanel /></div><div hidden={view !== 'missions'}><MissionsPanel active={view === 'missions'} /></div></>
    }
    render(<Panels />)
    const row = await screen.findByTestId('trigger-row-watcher')
    expect(within(row).getByText('Enabled')).toBeVisible()
    act(() => useStore.setState({ viewMode: 'missions' }))
    fireEvent.click(await screen.findByRole('button', { name: task.fields.title }))
    const heading = detail().getByRole('heading', { level: 2 })
    expect(await detail().findByRole('button', { name: 'Review watcher · schedule · Enabled' })).toBeVisible()
    if (action === 'agent close') act(close)
    else fireEvent.click(detail().getByRole('button', { name: action }))
    const link = await detail().findByRole('button', { name: 'Review watcher · schedule · Disabled' })
    expect(detail().getByRole('heading', { level: 2 })).toBe(heading)
    expect(useStore.getState().triggersSession.triggers[0]).toMatchObject({ enabled: false, revision: '2' })
    fireEvent.click(link)
    expect(screen.getByTestId('trigger-row-watcher')).toBe(row)
    expect(within(row).getByText('Disabled')).toBeVisible()
})
