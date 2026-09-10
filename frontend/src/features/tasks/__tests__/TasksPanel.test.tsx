import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TasksPanel } from '../TasksPanel'
import { useStore } from '@/store'

const fields = { title: 'Deliver search', description: '', stage: 'ready', archived: false }
let task: { id: string; revision: number; fields: typeof fields; activity: unknown[] }
let calls: { url: string; body: Record<string, unknown> }[]
beforeEach(() => {
    window.innerWidth = 1440
    task = { id: 'task-1', revision: 1, fields: { ...fields }, activity: [] }
    calls = []
    useStore.setState({ activeProjectPath: '/project', viewMode: 'tasks' })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        if (init?.body) {
            const body = JSON.parse(String(init.body)); calls.push({ url, body })
            if (init.method !== 'POST' && body.revision !== task.revision) return { ok: false, status: 409, json: async () => ({ detail: 'Conflict' }) }
            task = { ...task, id: init.method === 'POST' ? 'task-new' : task.id, revision: init.method === 'POST' ? 1 : task.revision + 1, fields: { ...task.fields, ...body.fields } }
            return { ok: true, json: async () => task }
        }
        return { ok: true, json: async () => ({ tasks: [task] }) }
    }))
})
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })

it('supports every manual stage, Done without a note, and reopening', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    for (const stage of ['backlog', 'planning', 'ready', 'in_progress', 'review', 'done', 'backlog']) {
        fireEvent.change(screen.getByLabelText('Stage'), { target: { value: stage } })
        fireEvent.click(screen.getByRole('button', { name: 'Save' }))
        await waitFor(() => expect(task.fields.stage).toBe(stage))
        await waitFor(() => expect(screen.getByRole('button', { name: 'Edit', exact: true })).toBeEnabled())
        fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    }
    expect(screen.getByText('Activity').closest('details')).not.toHaveAttribute('open')
    expect(screen.queryByLabelText('Priority')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Note / completion evidence')).not.toBeInTheDocument()
})

it('preserves drafts across refresh and tab activation, and reconciles concurrent changes', async () => {
    const view = render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'My draft' } })
    task = { ...task, revision: 2, fields: { ...task.fields, description: 'Server next action' } }
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Your edits are preserved')
    view.rerender(<TasksPanel active={false} />)
    view.rerender(<TasksPanel active />)
    expect(await screen.findByRole('status')).toHaveTextContent('newer revision')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Reconcile with latest revision' }))
    expect(screen.getByLabelText('Description')).toHaveValue('Server next action')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(task.revision).toBe(3))
    expect(task.fields.description).toBe('Server next action')
})

it('polls only while visible and preserves drafts when opening other cards', async () => {
    const view = render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unfinished edit' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Unfinished note' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Unfinished edit')
    expect(screen.getByLabelText('Description')).toHaveValue('Unfinished note')
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
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A draft' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Project A note' } })
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    await act(async () => { useStore.setState({ activeProjectPath: '/project' }) })
    expect(screen.getByLabelText('Title')).toHaveValue('Project A draft')
    expect(screen.getByLabelText('Description')).toHaveValue('Project A note')
})

it('creates a manual task and supports archival through accessible controls', async () => {
    render(<TasksPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Manual outcome' } })
    fireEvent.click(within(screen.getByRole('region', { name: 'Task details' })).getByRole('button', { name: 'Create task' }))
    expect(await screen.findByRole('button', { name: /Manual outcome/ })).toBeInTheDocument()
    expect(task.fields.stage).toBe('backlog')
    expect(calls[0].body.revision).toBeUndefined()
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.click(screen.getByRole('button', { name: 'Archive task' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: /Manual outcome/ })).not.toBeInTheDocument())
    fireEvent.click(screen.getByRole('button', { name: 'Show archived' }))
    expect(screen.getByRole('button', { name: /Manual outcome/ })).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore task' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Restore task' }))
    await waitFor(() => expect(task.fields.archived).toBe(false))
})

const editor = () => within(screen.getByRole('region', { name: 'Task details' }))
it('creates with only a title, keeps the saved task open and resets its baseline', async () => {
    render(<TasksPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    expect(screen.getByLabelText('Title')).toHaveFocus()
    expect(screen.getByLabelText('Title').tagName).toBe('INPUT')
    expect(screen.getByLabelText('Title')).toHaveAttribute('data-slot', 'input')
    expect(screen.getByLabelText('Stage')).toHaveAttribute('data-slot', 'native-select')
    expect(screen.getByLabelText('Description')).toHaveAttribute('data-slot', 'textarea')
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

it('preserves a new draft on Cancel', async () => {
    render(<TasksPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Draft title' } })
    fireEvent.click(editor().getByRole('button', { name: 'Cancel' }))
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    expect(screen.getByLabelText('Title')).toHaveValue('Draft title')
})

it('tracks reverted fields and note-only changes and discards to the latest revision', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    expect(screen.getByLabelText('Title')).toHaveFocus()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Changed' } })
    expect(editor().getByRole('heading', { name: 'Deliver search' })).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Deliver search' } })
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Note only' } })
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Description')).toHaveValue('Note only')
    task = { ...task, revision: 2, fields: { ...task.fields, title: 'Server title' } }
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await screen.findByRole('status')
    fireEvent.click(editor().getByRole('button', { name: 'Discard changes' }))
    expect(editor().getByRole('heading', { name: 'Server title' })).toHaveFocus()
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
})

it('locks editing and dismissal during saving and preserves input after failure', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Keep this' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Keep note' } })
    let reject!: (error: Error) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail }))
    fireEvent.click(editor().getByRole('button', { name: 'Save' }))
    expect(screen.getByLabelText('Title')).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Close' })).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Discard changes' })).toBeDisabled()
    fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
    expect(screen.getByLabelText('Title')).toBeInTheDocument()
    await act(async () => { reject(new Error('Save unavailable')) })
    expect(await screen.findByRole('alert')).toHaveTextContent('Save unavailable')
    expect(screen.getByLabelText('Title')).toHaveValue('Keep this')
    expect(screen.getByLabelText('Description')).toHaveValue('Keep note')
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
})

it('replaces the board at the narrow breakpoint and respects handled and outside Escape', async () => {
    window.innerWidth = 1024
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
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
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Conflicting edit' } })
    vi.mocked(fetch).mockResolvedValueOnce({ ok: false, status: 409, json: async () => ({}) } as Response)
    vi.mocked(fetch).mockRejectedValueOnce(new Error('Refresh unavailable'))
    fireEvent.click(editor().getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(editor().getByRole('button', { name: 'Close' })).toBeEnabled())
    fireEvent.click(editor().getByRole('button', { name: 'Close' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Conflicting edit')
    expect(screen.getByRole('status')).toHaveTextContent('reconcile before saving')
    expect(editor().getByRole('button', { name: 'Save' })).toBeDisabled()
    fireEvent.click(editor().getByRole('button', { name: 'Discard changes' }))
    expect(editor().getByRole('button', { name: 'Edit', exact: true })).toBeEnabled()
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
})

it.each((['creation', 'update', 'failure', 'conflict'] as const).flatMap(outcome => [true, false].map(returnWhilePending => ({ outcome, returnWhilePending }))))('owns a deferred $outcome across projects (return while pending: $returnWhilePending)', async ({ outcome, returnWhilePending }) => {
    render(<TasksPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: outcome === 'creation' ? 'Create task' : /Deliver search/ }))
    if (outcome !== 'creation') fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A saved title' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Project A evidence' } })
    let complete!: (response: Response) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
    fireEvent.click(editor().getByRole('button', { name: outcome === 'creation' ? 'Create task' : 'Save' }))
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
        expect(editor().getByRole('button', { name: outcome === 'creation' ? 'Cancel' : 'Close' })).toBeDisabled()
        expect(editor().getByRole('button', { name: 'Saving…' })).toBeDisabled()
        fireEvent.click(editor().getByRole('button', { name: 'Saving…' }))
        fireEvent.keyDown(screen.getByLabelText('Title'), { key: 'Escape' })
        expect(submissions()).toHaveLength(1)
        expect(screen.getByLabelText('Title')).toBeInTheDocument()
        await switchProject('/other')
    }
    const success = outcome === 'creation' || outcome === 'update'
    if (success) task = { ...task, id: outcome === 'creation' ? 'task-new' : task.id, revision: outcome === 'creation' ? 1 : 2, fields: { ...task.fields, title: 'Project A saved title', description: 'Project A evidence' } }
    if (outcome === 'conflict') task = { ...task, revision: 2, fields: { ...task.fields, stage: 'review' } }
    await act(async () => { complete({ ok: success, status: success ? 200 : outcome === 'conflict' ? 409 : 500, json: async () => success ? task : { detail: 'Save unavailable' } } as Response) })
    expect(screen.getByLabelText('Title')).toHaveValue('Project B draft')
    expect(screen.getByLabelText('Title')).toBeEnabled()
    await switchProject('/project')
    if (!success) {
        expect(screen.getByLabelText('Title')).toHaveValue('Project A saved title')
        expect(screen.getByLabelText('Title')).toBeEnabled()
    }
    expect(editor().getByRole('button', { name: outcome === 'creation' && !success ? 'Cancel' : 'Close' })).toBeEnabled()
    if (success) {
        expect(editor().getByRole('heading', { name: 'Project A saved title' })).toBeInTheDocument()
        expect(editor().getByRole('button', { name: 'Edit', exact: true })).toBeEnabled()
        expect(editor().getByText('Project A evidence')).toBeInTheDocument()
        expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
        fireEvent.click(editor().getByRole('button', { name: 'Close' }))
        fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
        expect(screen.getByLabelText('Title')).toHaveValue('')
    } else {
        expect(screen.getByLabelText('Description')).toHaveValue('Project A evidence')
        expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
        expect(screen.getByRole('alert')).toHaveTextContent(outcome === 'conflict' ? 'Your edits are preserved' : 'Save unavailable')
        if (outcome === 'conflict') {
            expect(editor().getByRole('button', { name: 'Save' })).toBeDisabled()
            fireEvent.click(editor().getByRole('button', { name: 'Reconcile with latest revision' }))
            expect(screen.getByLabelText('Stage')).toHaveValue('review')
            expect(screen.getByLabelText('Title')).toHaveValue('Project A saved title')
        }
        expect(editor().getByRole('button', { name: 'Save' })).toBeEnabled()
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

it('archives separately without saving or losing edited fields', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unsaved title' } })
    fireEvent.click(screen.getByRole('button', { name: 'Archive task' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore task' })).toBeEnabled())
    expect(calls[0].body.fields).toEqual({ archived: true })
    expect(task.fields.title).toBe('Deliver search')
    expect(screen.getByLabelText('Title')).toHaveValue('Unsaved title')
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() => expect(task.fields.title).toBe('Unsaved title'))
})

it('filters titles locally with matching counts and keeps the selected read view', async () => {
    task.fields.description = 'Paragraph one.\n\nParagraph two.'
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(editor().getByRole('heading')).toHaveFocus()
    expect(editor().getByText(task.fields.description, { normalizer: value => value })).toHaveClass('whitespace-pre-wrap')
    const count = vi.mocked(fetch).mock.calls.length
    fireEvent.change(screen.getByLabelText('Search titles'), { target: { value: 'DELIVER' } })
    expect(within(screen.getByRole('region', { name: 'Ready' })).getByLabelText('1 matching tasks')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Search titles'), { target: { value: 'missing' } })
    expect(screen.queryByRole('button', { name: 'Deliver search' })).not.toBeInTheDocument()
    expect(screen.getAllByText('No matches')).toHaveLength(6)
    expect(editor().getByRole('heading')).toHaveTextContent('Deliver search')
    fireEvent.click(screen.getByRole('button', { name: 'Clear search' }))
    expect(screen.getByRole('button', { name: 'Deliver search' })).toHaveAttribute('aria-pressed', 'true')
    expect(fetch).toHaveBeenCalledTimes(count)
    task = { ...task, revision: 2, fields: { ...task.fields, archived: true } }
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(editor().getByText('Archived')).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: 'Deliver search' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Show archived' }))
    expect(screen.getByRole('button', { name: 'Show archived' })).toHaveAttribute('aria-pressed', 'true')
    expect(within(screen.getByRole('button', { name: 'Deliver search' })).getByText('Archived')).toBeInTheDocument()
})

it('updates only stage from read mode, preserves prior stage on failure, and requires retry after conflict', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.change(screen.getByLabelText('Stage'), { target: { value: 'done' } })
    await waitFor(() => expect(screen.getByRole('button', { name: 'Edit', exact: true })).toBeEnabled())
    expect(calls[0].body).toEqual({ revision: 1, fields: { stage: 'done' }, actor: 'human' })
    expect(screen.getByLabelText('Stage')).toHaveValue('done')
    vi.mocked(fetch).mockRejectedValueOnce(new Error('Stage unavailable'))
    fireEvent.change(screen.getByLabelText('Stage'), { target: { value: 'backlog' } })
    expect(await screen.findByRole('alert')).toHaveTextContent('Stage unavailable')
    expect(screen.getByLabelText('Stage')).toHaveValue('done')
    task = { ...task, revision: 3, fields: { ...task.fields, stage: 'review', title: 'Latest title' } }
    fireEvent.change(screen.getByLabelText('Stage'), { target: { value: 'backlog' } })
    await waitFor(() => expect(screen.getByLabelText('Stage')).toHaveValue('review'))
    expect(screen.getByRole('alert')).toHaveTextContent('retry your stage change')
    expect(editor().getByRole('heading')).toHaveTextContent('Latest title')
    expect(calls).toHaveLength(2)
    fireEvent.change(screen.getByLabelText('Stage'), { target: { value: 'backlog' } })
    await waitFor(() => expect(screen.getByLabelText('Stage')).toHaveValue('backlog'))
    expect(calls[2].body.revision).toBe(3)
})

it('discards new creation explicitly and does not resurrect reverted cached drafts', async () => {
    render(<TasksPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Temporary' } })
    fireEvent.click(screen.getByRole('button', { name: 'Discard changes' }))
    expect(screen.queryByRole('region', { name: 'Task details' })).not.toBeInTheDocument()
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    expect(screen.getByLabelText('Title')).toHaveFocus()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Temporary' } })
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    fireEvent.click(screen.getByRole('button', { name: 'Deliver search' }))
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Deliver search' } })
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    fireEvent.click(screen.getByRole('button', { name: 'Deliver search' }))
    expect(screen.getByRole('button', { name: 'Edit', exact: true })).toBeInTheDocument()
})

it('keeps a pending stage change owned by its project and locks read actions', async () => {
    render(<TasksPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    let complete!: (response: Response) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
    fireEvent.change(screen.getByLabelText('Stage'), { target: { value: 'done' } })
    expect(screen.getByLabelText('Stage')).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Edit', exact: true })).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Close' })).toBeDisabled()
    fireEvent.keyDown(editor().getByRole('heading'), { key: 'Escape' })
    expect(editor().getByRole('status')).toHaveTextContent('Saving')
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Other project draft' } })
    task = { ...task, revision: 2, fields: { ...task.fields, stage: 'done' } }
    await act(async () => { complete({ ok: true, json: async () => task } as Response) })
    expect(screen.getByLabelText('Title')).toHaveValue('Other project draft')
    await act(async () => { useStore.setState({ activeProjectPath: '/project' }) })
    expect(screen.getByLabelText('Stage')).toHaveValue('done')
    expect(editor().getByRole('button', { name: 'Edit', exact: true })).toBeEnabled()
})
