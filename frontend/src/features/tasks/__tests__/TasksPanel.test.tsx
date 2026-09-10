import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TasksPanel } from '../TasksPanel'
import { useStore } from '@/store'
import { selectSelectedRunId } from '@/state/runsSessionSelectors'

const fields = { title: 'Deliver search', description: '', acceptance_criteria: '', next_action: '', stage: 'ready', priority: 2, blocked: '', needs_input: '', archived: false, conversations: [] as string[], artifacts: [] as string[], runs: [{ run_id: 'run-1', stage: 'planning' }] }
let task: { id: string; revision: number; fields: typeof fields; activity: unknown[] }
let attention: { task_id: string; run_id: string }[]
let calls: { url: string; body: Record<string, unknown> }[]
beforeEach(() => {
    window.innerWidth = 1440
    task = { id: 'task-1', revision: 1, fields: { ...fields }, activity: [] }
    attention = []; calls = []
    useStore.setState({ activeProjectPath: '/project', viewMode: 'tasks' })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        if (init?.body) {
            const body = JSON.parse(String(init.body)); calls.push({ url, body })
            if (init.method !== 'POST' && body.revision !== task.revision) return { ok: false, status: 409, json: async () => ({ detail: 'Conflict' }) }
            task = { ...task, id: init.method === 'POST' ? 'task-new' : task.id, revision: init.method === 'POST' ? 1 : task.revision + 1, fields: { ...task.fields, ...body.fields } }
            return { ok: true, json: async () => task }
        }
        return { ok: true, json: async () => ({ tasks: [task], runs: [{ run_id: 'run-1', status: 'completed' }], attention }) }
    }))
})
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })

it('supports labelled editing, completion evidence, and actual run status without advancing stage', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search.*Normal/ }))
    expect(screen.getByLabelText('Stage')).toHaveValue('ready')
    expect(screen.getByRole('button', { name: 'Open run run-1 — completed' })).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Stage'), { target: { value: 'done' } })
    expect(screen.getByLabelText('Note / completion evidence')).toBeRequired()
    fireEvent.click(screen.getByRole('button', { name: 'Save task' }))
    expect(calls).toHaveLength(0)
    fireEvent.change(screen.getByLabelText('Note / completion evidence'), { target: { value: 'Manual acceptance review passed' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save task' }))
    await waitFor(() => expect(task.fields.stage).toBe('done'))
    expect(calls[0].body.note).toBe('Manual acceptance review passed')
    expect(calls[0].url).toContain('project_path=%2Fproject')
})

it('preserves drafts across refresh and tab activation, and reconciles concurrent changes', async () => {
    const view = render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search.*Normal/ }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'My draft' } })
    task = { ...task, revision: 2, fields: { ...task.fields, next_action: 'Server next action' } }
    fireEvent.click(screen.getByRole('button', { name: 'Save task' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Your edits are preserved')
    view.rerender(<TasksPanel active={false} />)
    view.rerender(<TasksPanel active />)
    expect(await screen.findByRole('status')).toHaveTextContent('newer revision')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    expect(screen.getByRole('button', { name: 'Save task' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Reconcile with latest revision' }))
    expect(screen.getByLabelText('Next action')).toHaveValue('Server next action')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    fireEvent.click(screen.getByRole('button', { name: 'Save task' }))
    await waitFor(() => expect(task.revision).toBe(3))
    expect(task.fields.next_action).toBe('Server next action')
})

it.each(['active', 'all'] as const)('opens a linked run in the effective Runs selection with %s scope', async (scopeMode) => {
    useStore.getState().updateRunsListSession({ scopeMode, selectedRunIdByScopeKey: { all: 'previous-all-run', 'project:/project': 'previous-project-run' } })
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Open run run-1 — completed' }))
    expect(useStore.getState().viewMode).toBe('runs')
    expect(selectSelectedRunId(useStore.getState())).toBe('run-1')
})

it.each(['active', 'all'] as const)('filters attention and selects the descendant question run with %s scope', async (scopeMode) => {
    useStore.getState().updateRunsListSession({ scopeMode, selectedRunIdByScopeKey: { all: 'previous-all-run', 'project:/project': 'previous-project-run' } })
    attention = [{ task_id: 'task-1', run_id: 'run-child' }]
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByLabelText('Needs attention'))
    expect(within(screen.getByRole('region', { name: 'Ready' })).getByRole('button', { name: /Deliver search/ })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Answer questions in run run-child' }))
    expect(useStore.getState().viewMode).toBe('runs')
    expect(selectSelectedRunId(useStore.getState())).toBe('run-child')
    attention = []
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(within(screen.getByRole('region', { name: 'Ready' })).queryByRole('button')).not.toBeInTheDocument())
    task = { ...task, fields: { ...task.fields, blocked: 'Waiting for review' } }
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    expect(await screen.findByRole('button', { name: /Blocked: Waiting for review/ })).toBeInTheDocument()
})

it('polls only while visible and preserves drafts when opening other cards', async () => {
    const view = render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unfinished edit' } })
    fireEvent.change(screen.getByLabelText('Note / completion evidence'), { target: { value: 'Unfinished note' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Unfinished edit')
    expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('Unfinished note')
    view.rerender(<TasksPanel active={false} />)
    vi.useFakeTimers()
    view.rerender(<TasksPanel active />)
    const count = vi.mocked(fetch).mock.calls.length
    await act(async () => { vi.advanceTimersByTime(15000) })
    expect(vi.mocked(fetch).mock.calls.length).toBe(count + 1)
    view.rerender(<TasksPanel active={false} />)
    await act(async () => { vi.advanceTimersByTime(30000) })
    expect(vi.mocked(fetch).mock.calls.length).toBe(count + 1)
})

it('keeps unsaved edits scoped to their project when switching projects', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A draft' } })
    fireEvent.change(screen.getByLabelText('Note / completion evidence'), { target: { value: 'Project A note' } })
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    await act(async () => { useStore.setState({ activeProjectPath: '/project' }) })
    expect(screen.getByLabelText('Title')).toHaveValue('Project A draft')
    expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('Project A note')
})

it('creates a manual task and supports archival through accessible controls', async () => {
    render(<TasksPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Manual outcome' } })
    fireEvent.change(screen.getByLabelText('Priority'), { target: { value: '1' } })
    fireEvent.click(within(screen.getByRole('region', { name: 'Task details' })).getByRole('button', { name: 'Create task' }))
    expect(await screen.findByRole('button', { name: /Manual outcome.*High/ })).toBeInTheDocument()
    expect(task.fields.runs).toEqual([])
    expect(calls[0].body.revision).toBeUndefined()
    fireEvent.click(screen.getByText('Archival'))
    fireEvent.click(screen.getByLabelText('Archived'))
    fireEvent.click(screen.getByRole('button', { name: 'Save task' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: /Manual outcome.*High/ })).not.toBeInTheDocument())
    fireEvent.click(screen.getByLabelText('Show archived'))
    expect(screen.getByRole('button', { name: /Manual outcome.*High.*Archived/ })).toBeInTheDocument()
})

const editor = () => within(screen.getByRole('region', { name: 'Task details' }))
it('creates with only a title, keeps the saved task open and resets its baseline', async () => {
    render(<TasksPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    expect(screen.getByLabelText('Title')).toHaveFocus()
    expect(screen.getByLabelText('Title').tagName).toBe('INPUT')
    expect(screen.getByLabelText('Title')).toHaveAttribute('data-slot', 'input')
    expect(screen.getByLabelText('Stage')).toHaveAttribute('data-slot', 'native-select')
    expect(screen.getByLabelText('Outcome and decisions')).toHaveAttribute('data-slot', 'textarea')
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    expect(screen.queryByText('Related resources')).not.toBeInTheDocument()
    expect(screen.queryByText('Activity')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Quick capture' } })
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.click(editor().getByRole('button', { name: 'Create task' }))
    await waitFor(() => expect(editor().getByRole('heading', { name: 'Quick capture' })).toHaveFocus())
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    expect(task.fields.title).toBe('Quick capture')
    expect(calls).toHaveLength(1)
})

it('preserves new fields and notes on close and explicitly discards the new draft', async () => {
    render(<TasksPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Draft capture' } })
    fireEvent.change(screen.getByLabelText('Note / completion evidence'), { target: { value: 'Draft note' } })
    fireEvent.click(editor().getByRole('button', { name: 'Close' }))
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    expect(screen.getByLabelText('Title')).toHaveValue('Draft capture')
    expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('Draft note')
    fireEvent.click(editor().getByRole('button', { name: 'Discard changes' }))
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    expect(screen.getByLabelText('Title')).toHaveValue('')
    expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('')
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    expect(calls).toHaveLength(0)
})

it('tracks reverted fields and note-only changes and discards to the latest revision', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    expect(editor().getByRole('heading', { name: 'Deliver search' })).toHaveFocus()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Changed' } })
    expect(editor().getByRole('heading', { name: 'Deliver search' })).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Deliver search' } })
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Note / completion evidence'), { target: { value: 'Note only' } })
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('Note only')
    task = { ...task, revision: 2, fields: { ...task.fields, title: 'Server title' } }
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await screen.findByRole('status')
    fireEvent.click(editor().getByRole('button', { name: 'Discard changes' }))
    expect(screen.getByLabelText('Title')).toHaveValue('Server title')
    expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('')
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
})

it('locks editing and dismissal during saving and preserves input after failure', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Keep this' } })
    fireEvent.change(screen.getByLabelText('Note / completion evidence'), { target: { value: 'Keep note' } })
    let reject!: (error: Error) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail }))
    fireEvent.click(editor().getByRole('button', { name: 'Save task' }))
    expect(screen.getByLabelText('Title')).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Close' })).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Discard changes' })).toBeDisabled()
    fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
    expect(screen.getByLabelText('Title')).toBeInTheDocument()
    await act(async () => { reject(new Error('Save unavailable')) })
    expect(await screen.findByRole('alert')).toHaveTextContent('Save unavailable')
    expect(screen.getByLabelText('Title')).toHaveValue('Keep this')
    expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('Keep note')
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
})

it('replaces the board at the narrow breakpoint and respects handled and outside Escape', async () => {
    window.innerWidth = 1024
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    expect(screen.queryByRole('region', { name: 'Ready' })).not.toBeInTheDocument()
    fireEvent.keyDown(screen.getByRole('heading', { name: 'Tasks' }), { key: 'Escape' })
    const title = screen.getByLabelText('Title')
    title.addEventListener('keydown', e => e.preventDefault(), { once: true })
    fireEvent.keyDown(title, { key: 'Escape' })
    expect(title).toBeInTheDocument()
    fireEvent.keyDown(title, { key: 'Escape' })
    expect(screen.getByRole('region', { name: 'Ready' })).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Tasks' })).toHaveFocus())
})

it('keeps conflicts blocked after dismissal when the latest revision cannot be loaded', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Conflicting edit' } })
    vi.mocked(fetch).mockResolvedValueOnce({ ok: false, status: 409, json: async () => ({}) } as Response)
    vi.mocked(fetch).mockRejectedValueOnce(new Error('Refresh unavailable'))
    fireEvent.click(editor().getByRole('button', { name: 'Save task' }))
    await waitFor(() => expect(editor().getByRole('button', { name: 'Close' })).toBeEnabled())
    fireEvent.click(editor().getByRole('button', { name: 'Close' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Conflicting edit')
    expect(screen.getByRole('status')).toHaveTextContent('reconcile before saving')
    expect(editor().getByRole('button', { name: 'Save task' })).toBeDisabled()
    fireEvent.click(editor().getByRole('button', { name: 'Discard changes' }))
    expect(editor().getByRole('button', { name: 'Save task' })).toBeDisabled()
    task = { ...task, revision: 2 }
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Reconcile with latest revision' }))
    expect(editor().getByRole('button', { name: 'Save task' })).toBeEnabled()
})

it('retains raw relationships and conversation navigation', async () => {
    task.fields = { ...task.fields, conversations: ['conversation-1'], artifacts: ['README.md'] }
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByText('Relationships'))
    expect(screen.getByLabelText('Stage supported by run-1')).toHaveValue('planning')
    fireEvent.change(screen.getByLabelText('Run IDs (one per line)'), { target: { value: 'run-1\nrun-2' } })
    fireEvent.change(screen.getByLabelText('Stage supported by run-2'), { target: { value: 'review' } })
    fireEvent.click(editor().getByRole('button', { name: 'Save task' }))
    await waitFor(() => expect(task.fields.runs).toEqual([{ run_id: 'run-1', stage: 'planning' }, { run_id: 'run-2', stage: 'review' }]))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Open conversation conversation-1' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Open conversation conversation-1' }))
    expect(useStore.getState().viewMode).toBe('home')
})

it.each((['creation', 'update', 'failure', 'conflict'] as const).flatMap(outcome => [true, false].map(returnWhilePending => ({ outcome, returnWhilePending }))))('owns a deferred $outcome across projects (return while pending: $returnWhilePending)', async ({ outcome, returnWhilePending }) => {
    render(<TasksPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: outcome === 'creation' ? 'Create task' : /Deliver search/ }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A saved title' } })
    fireEvent.change(screen.getByLabelText('Note / completion evidence'), { target: { value: 'Project A evidence' } })
    let complete!: (response: Response) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
    fireEvent.click(editor().getByRole('button', { name: outcome === 'creation' ? 'Create task' : 'Save task' }))
    const submissions = () => vi.mocked(fetch).mock.calls.filter(([, init]) => init?.body)
    expect(submissions()).toHaveLength(1)
    expect(submissions()[0][0]).toContain('project_path=%2Fproject')
    const switchProject = async (project: string) => { await act(async () => { useStore.setState({ activeProjectPath: project }) }) }
    await switchProject('/other')
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project B draft' } })
    if (returnWhilePending) {
        await switchProject('/project')
        expect(screen.getByLabelText('Title')).toHaveValue('Project A saved title')
        expect(screen.getByLabelText('Title')).toBeDisabled()
        expect(editor().getByRole('button', { name: 'Close' })).toBeDisabled()
        expect(editor().getByRole('button', { name: 'Saving…' })).toBeDisabled()
        fireEvent.click(editor().getByRole('button', { name: 'Saving…' }))
        fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
        expect(submissions()).toHaveLength(1)
        expect(screen.getByLabelText('Title')).toBeInTheDocument()
        await switchProject('/other')
    }
    const success = outcome === 'creation' || outcome === 'update'
    if (success) task = { ...task, id: outcome === 'creation' ? 'task-new' : task.id, revision: outcome === 'creation' ? 1 : 2, fields: { ...task.fields, title: 'Project A saved title' } }
    if (outcome === 'conflict') task = { ...task, revision: 2, fields: { ...task.fields, next_action: 'Server next action' } }
    await act(async () => { complete({ ok: success, status: success ? 200 : outcome === 'conflict' ? 409 : 500, json: async () => success ? task : { detail: 'Save unavailable' } } as Response) })
    expect(screen.getByLabelText('Title')).toHaveValue('Project B draft')
    expect(screen.getByLabelText('Title')).toBeEnabled()
    await switchProject('/project')
    expect(screen.getByLabelText('Title')).toHaveValue('Project A saved title')
    expect(screen.getByLabelText('Title')).toBeEnabled()
    expect(editor().getByRole('button', { name: 'Close' })).toBeEnabled()
    if (success) {
        expect(editor().getByRole('heading', { name: 'Project A saved title' })).toBeInTheDocument()
        expect(editor().getByRole('button', { name: 'Save task' })).toBeEnabled()
        expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('')
        expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
        fireEvent.click(editor().getByRole('button', { name: 'Close' }))
        fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
        expect(screen.getByLabelText('Title')).toHaveValue('')
    } else {
        expect(screen.getByLabelText('Note / completion evidence')).toHaveValue('Project A evidence')
        expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
        expect(screen.getByRole('alert')).toHaveTextContent(outcome === 'conflict' ? 'Your edits are preserved' : 'Save unavailable')
        if (outcome === 'conflict') {
            expect(editor().getByRole('button', { name: 'Save task' })).toBeDisabled()
            fireEvent.click(editor().getByRole('button', { name: 'Reconcile with latest revision' }))
            expect(screen.getByLabelText('Next action')).toHaveValue('Server next action')
            expect(screen.getByLabelText('Title')).toHaveValue('Project A saved title')
        }
        expect(editor().getByRole('button', { name: 'Save task' })).toBeEnabled()
    }
    expect(submissions()).toHaveLength(1)
})


it('polls only the selected project while retained controllers keep their drafts', async () => {
    const view = render(<TasksPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    vi.useFakeTimers()
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    vi.mocked(fetch).mockClear()
    await act(async () => { vi.advanceTimersByTime(30000) })
    expect(vi.mocked(fetch).mock.calls.map(([url]) => url)).toEqual([
        '/workspace/api/tasks?project_path=%2Fother', '/workspace/api/tasks?project_path=%2Fother',
    ])
    view.rerender(<TasksPanel active={false} />)
    vi.mocked(fetch).mockClear()
    await act(async () => { vi.advanceTimersByTime(30000) })
    expect(fetch).not.toHaveBeenCalled()
})
