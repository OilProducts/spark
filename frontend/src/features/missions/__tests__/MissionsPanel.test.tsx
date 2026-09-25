import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { MissionsPanel } from '../MissionsPanel'
import { useStore } from '@/store'

const fields = { title: 'Deliver search', description: '', stage: 'ready', archived: false }
let task: { id: string; revision: number; fields: typeof fields; activity: unknown[] }
let calls: { url: string; body: Record<string, unknown> }[]
beforeEach(() => {
    window.innerWidth = 1440
    task = { id: 'task-1', revision: 1, fields: { ...fields }, activity: [] }
    calls = []
    useStore.setState({ activeProjectPath: '/project', viewMode: 'missions' })
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        if (init?.body) {
            const body = JSON.parse(String(init.body)); calls.push({ url, body })
            if (init.method !== 'POST' && body.revision !== task.revision) return { ok: false, status: 409, json: async () => ({ detail: 'Conflict' }) }
            task = { ...task, id: init.method === 'POST' ? 'task-new' : task.id, revision: init.method === 'POST' ? 1 : task.revision + 1, fields: { ...task.fields, ...body.fields } }
            return { ok: true, json: async () => task }
        }
        if (url.includes('/events')) return { ok: true, json: async () => ({ events: [] }) }
        return { ok: true, json: async () => ({ missions: [task] }) }
    }))
})
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })

it('supports every manual stage, Done without a note, and reopening', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    for (const stage of ['backlog', 'planning', 'ready', 'in_progress', 'review', 'done', 'backlog']) {
        fireEvent.change(screen.getByLabelText('Stage'), { target: { value: stage } })
        fireEvent.click(screen.getByRole('button', { name: /^Save/ }))
        await waitFor(() => expect(task.fields.stage).toBe(stage))
        await waitFor(() => expect(screen.getByRole('button', { name: 'Edit', exact: true })).toBeEnabled())
        fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    }
    expect(screen.getByText('Activity').closest('details')).not.toHaveAttribute('open')
    expect(screen.queryByLabelText('Priority')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('Note / completion evidence')).not.toBeInTheDocument()
})

it('preserves drafts across refresh and tab activation, and reconciles concurrent changes', async () => {
    const view = render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'My draft' } })
    task = { ...task, revision: 2, fields: { ...task.fields, description: 'Server next action' } }
    fireEvent.click(screen.getByRole('button', { name: /^Save/ }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Your edits are preserved')
    view.rerender(<MissionsPanel active={false} />)
    view.rerender(<MissionsPanel active />)
    expect(await screen.findByRole('status')).toHaveTextContent('newer revision')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
    fireEvent.click(screen.getByRole('button', { name: 'Reconcile with latest revision' }))
    expect(screen.getByLabelText('Description')).toHaveValue('Server next action')
    expect(screen.getByLabelText('Title')).toHaveValue('My draft')
    fireEvent.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(task.revision).toBe(3))
    expect(task.fields.description).toBe('Server next action')
})

it('loads on activation without polling and preserves drafts when opening other cards', async () => {
    const view = render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unfinished edit' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Unfinished note' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Unfinished edit')
    expect(screen.getByLabelText('Description')).toHaveValue('Unfinished note')
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
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A draft' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Project A note' } })
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    await act(async () => { useStore.setState({ activeProjectPath: '/project' }) })
    expect(screen.getByLabelText('Title')).toHaveValue('Project A draft')
    expect(screen.getByLabelText('Description')).toHaveValue('Project A note')
})

it('creates a manual mission and supports archival through accessible controls', async () => {
    render(<MissionsPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Manual outcome' } })
    fireEvent.click(within(screen.getByRole('region', { name: 'Mission details' })).getByRole('button', { name: 'Create mission' }))
    expect(await screen.findByRole('button', { name: /Manual outcome/ })).toBeInTheDocument()
    expect(task.fields.stage).toBe('backlog')
    expect(calls[0].body.revision).toBeUndefined()
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.click(screen.getByRole('button', { name: 'Archive mission' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: /Manual outcome/ })).not.toBeInTheDocument())
    fireEvent.click(screen.getByRole('button', { name: 'Show archived' }))
    expect(screen.getByRole('button', { name: /Manual outcome/ })).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore mission' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Restore mission' }))
    await waitFor(() => expect(task.fields.archived).toBe(false))
})

const editor = () => within(screen.getByRole('region', { name: 'Mission details' }))
it('creates with only a title, keeps the saved mission open and resets its baseline', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
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
    fireEvent.click(editor().getByRole('button', { name: 'Create mission' }))
    await waitFor(() => expect(editor().getByRole('heading', { name: 'Quick capture' })).toHaveFocus())
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
    expect(task.fields.title).toBe('Quick capture')
    expect(calls).toHaveLength(1)
})

it('preserves a new draft on Cancel', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Draft title' } })
    fireEvent.click(editor().getByRole('button', { name: 'Cancel' }))
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    expect(screen.getByLabelText('Title')).toHaveValue('Draft title')
})

it('tracks reverted fields and note-only changes and discards to the latest revision', async () => {
    render(<MissionsPanel active />)
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
    fireEvent.click(editor().getByRole('button', { name: /^Discard/ }))
    expect(editor().getByRole('heading', { name: 'Server title' })).toHaveFocus()
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument()
})

it('locks editing and dismissal during saving and preserves input after failure', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Keep this' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Keep note' } })
    let reject!: (error: Error) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail }))
    fireEvent.click(editor().getByRole('button', { name: /^Save/ }))
    expect(screen.getByLabelText('Title')).toBeDisabled()
    expect(editor().getByRole('button', { name: 'Close' })).toBeDisabled()
    expect(editor().getByRole('button', { name: /^Discard/ })).toBeDisabled()
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
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    expect(screen.queryByRole('region', { name: 'Ready' })).not.toBeInTheDocument()
    fireEvent.keyDown(screen.getByRole('heading', { name: 'Missions' }), { key: 'Escape' })
    const title = screen.getByLabelText('Title')
    title.addEventListener('keydown', e => e.preventDefault(), { once: true })
    fireEvent.keyDown(title, { key: 'Escape' })
    expect(title).toBeInTheDocument()
    fireEvent.keyDown(title, { key: 'Escape' })
    expect(screen.getByRole('region', { name: 'Ready' })).toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('heading', { name: 'Missions' })).toHaveFocus())
})

it('keeps conflicts blocked after dismissal when the latest revision cannot be loaded', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: /Deliver search/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
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
    expect(editor().getByRole('button', { name: 'Edit', exact: true })).toBeEnabled()
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
})

it.each((['creation', 'update', 'failure', 'conflict'] as const).flatMap(outcome => [true, false].map(returnWhilePending => ({ outcome, returnWhilePending }))))('owns a deferred $outcome across projects (return while pending: $returnWhilePending)', async ({ outcome, returnWhilePending }) => {
    render(<MissionsPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: outcome === 'creation' ? 'Create mission' : /Deliver search/ }))
    if (outcome !== 'creation') fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Project A saved title' } })
    fireEvent.change(screen.getByLabelText('Description'), { target: { value: 'Project A evidence' } })
    let complete!: (response: Response) => void
    vi.mocked(fetch).mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
    fireEvent.click(editor().getByRole('button', { name: outcome === 'creation' ? 'Create mission' : /^Save/ }))
    const submissions = () => vi.mocked(fetch).mock.calls.filter(([, init]) => init?.body)
    expect(submissions()).toHaveLength(1)
    expect(submissions()[0][0]).toContain('project_path=%2Fproject')
    const switchProject = async (project: string) => { await act(async () => { useStore.setState({ activeProjectPath: project }) }) }
    await switchProject('/other')
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
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
        fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
        expect(screen.getByLabelText('Title')).toHaveValue('')
    } else {
        expect(screen.getByLabelText('Description')).toHaveValue('Project A evidence')
        expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
        expect(screen.getByRole('alert')).toHaveTextContent(outcome === 'conflict' ? 'Your edits are preserved' : 'Save unavailable')
        if (outcome === 'conflict') {
            expect(editor().getByRole('button', { name: /^Save/ })).toBeDisabled()
            fireEvent.click(editor().getByRole('button', { name: 'Reconcile with latest revision' }))
            expect(screen.getByLabelText('Stage')).toHaveValue('review')
            expect(screen.getByLabelText('Title')).toHaveValue('Project A saved title')
        }
        expect(editor().getByRole('button', { name: /^Save/ })).toBeEnabled()
    }
    expect(submissions()).toHaveLength(1)
})


const live = (projectPath: string, mission: unknown) => act(async () => {
    window.dispatchEvent(new CustomEvent('spark:mission-live-event', { detail: { projectPath, mission } }))
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
    await live('/other', { ...task, id: 'task-live', fields: { ...task.fields, title: 'Arrived live' } })
    expect(screen.getByRole('button', { name: 'Arrived live' })).toBeInTheDocument()
    vi.mocked(fetch).mockClear()
    await live('/other', null)
    expect(vi.mocked(fetch).mock.calls.map(([url]) => url)).toEqual(['/workspace/api/missions?project_path=%2Fother'])
    view.rerender(<MissionsPanel active={false} />)
    vi.mocked(fetch).mockClear()
    await live('/other', null)
    await act(async () => { vi.advanceTimersByTime(30000) })
    expect(fetch).not.toHaveBeenCalled()
})

it('archives separately without saving or losing edited fields', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    fireEvent.click(screen.getByRole('button', { name: 'Edit', exact: true }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Unsaved title' } })
    fireEvent.click(screen.getByRole('button', { name: 'Archive mission' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restore mission' })).toBeEnabled())
    expect(calls[0].body.fields).toEqual({ archived: true })
    expect(task.fields.title).toBe('Deliver search')
    expect(screen.getByLabelText('Title')).toHaveValue('Unsaved title')
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(task.fields.title).toBe('Unsaved title'))
})

it('filters titles locally with matching counts and keeps the selected read view', async () => {
    task.fields.description = 'Paragraph one.\n\nParagraph two.'
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(editor().getByRole('heading')).toHaveFocus()
    expect(editor().getByText(task.fields.description, { normalizer: value => value })).toHaveClass('whitespace-pre-wrap')
    const count = vi.mocked(fetch).mock.calls.length
    fireEvent.change(screen.getByLabelText('Search titles'), { target: { value: 'DELIVER' } })
    expect(within(screen.getByRole('region', { name: 'Ready' })).getByLabelText('1 matching missions')).toBeInTheDocument()
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
    render(<MissionsPanel active />)
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
    render(<MissionsPanel active />)
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Temporary' } })
    fireEvent.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(screen.queryByRole('region', { name: 'Mission details' })).not.toBeInTheDocument()
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
    render(<MissionsPanel active />)
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
    fireEvent.click(screen.getByRole('button', { name: 'Create mission' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Other project draft' } })
    task = { ...task, revision: 2, fields: { ...task.fields, stage: 'done' } }
    await act(async () => { complete({ ok: true, json: async () => task } as Response) })
    expect(screen.getByLabelText('Title')).toHaveValue('Other project draft')
    await act(async () => { useStore.setState({ activeProjectPath: '/project' }) })
    expect(screen.getByLabelText('Stage')).toHaveValue('done')
    expect(editor().getByRole('button', { name: 'Edit', exact: true })).toBeEnabled()
})

const started = () => ({
    ...task, revision: 2, fields: { ...task.fields, stage: 'in_progress', hooks: [{ on: 'run.completed', label: 'build', do: 'ignore' }], budget: { concurrent_runs: 4, total_runs: 25, reactions: 10 } },
    started_at: '2026-09-23', paused: false, closed: null, cursor: 2, state: '## Progress\n\nBuild **running**',
    execution: { substate: 'running', reason: '1 run(s) in flight' },
    runs: [{ run_id: 'run-build', label: 'build', role: 'work', launched_at: 't', launched_by_event: 'e', status: 'running' }, { run_id: 'run-react', label: 'reaction', role: 'reaction', launched_at: 't', launched_by_event: 'e', status: 'completed' }],
})
it('starts a mission with the only pre-start control', async () => {
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    const controls = within(editor().getByRole('region', { name: 'Controls' }))
    expect(controls.getAllByRole('button').map(button => button.textContent)).toEqual(['Start'])
    expect(editor().queryByRole('region', { name: 'State' })).not.toBeInTheDocument()
    expect(screen.queryByTestId('mission-execution-chip')).not.toBeInTheDocument()
    vi.mocked(fetch).mockImplementationOnce(async (url, init) => { calls.push({ url: String(url), body: JSON.parse(String(init?.body)) }); task = started() as typeof task; return { ok: true, json: async () => task } as Response })
    fireEvent.click(controls.getByRole('button', { name: 'Start' }))
    expect(await editor().findByRole('region', { name: 'State' })).toBeInTheDocument()
    expect(calls[0].url).toBe('/workspace/api/missions/task-1/start?project_path=%2Fproject')
    expect(within(screen.getByRole('region', { name: 'In progress' })).getByTestId('mission-execution-chip')).toHaveTextContent('Running')
    expect(screen.getByText('1 in flight')).toBeInTheDocument()
})

it('shows state, runs, events, and hooks for a started mission and drives its controls', async () => {
    task = started() as typeof task
    const events = [{ seq: 1, id: 'a', at: 't', kind: 'mission.started', source: 'human', payload: {} }, { seq: 2, id: 'b', at: 't', kind: 'human.message', source: 'human', payload: { message: 'Prefer small diffs' } }]
    vi.mocked(fetch).mockImplementation(async (url, init) => {
        const text = String(url)
        if (init?.body) {
            const body = JSON.parse(String(init.body)); calls.push({ url: text, body })
            if (text.includes('/pause')) task = { ...task, paused: true } as typeof task
            if (body.yaml) task = { ...task, revision: task.revision + 1 }
            return { ok: true, json: async () => task } as Response
        }
        if (text.includes('/events')) return { ok: true, json: async () => ({ events }) } as Response
        return { ok: true, json: async () => ({ missions: [task] }) } as Response
    })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(within(editor().getByRole('region', { name: 'State' })).getByText('running').tagName).toBe('STRONG')
    expect(await editor().findByText('Prefer small diffs')).toBeInTheDocument()
    expect(vi.mocked(fetch).mock.calls.some(([url]) => url === '/workspace/api/missions/task-1/events?project_path=%2Fproject')).toBe(true)
    fireEvent.change(editor().getByLabelText('Message'), { target: { value: 'Ship it' } })
    fireEvent.click(editor().getByRole('button', { name: 'Send message' }))
    await waitFor(() => expect(calls.at(-1)).toEqual({ url: '/workspace/api/missions/task-1/events?project_path=%2Fproject', body: { kind: 'human.message', payload: { message: 'Ship it' } } }))
    await waitFor(() => expect(editor().getByLabelText('Message')).toHaveValue(''))
    const yaml = editor().getByLabelText('YAML') as HTMLTextAreaElement
    expect(yaml.value).toBe('hooks:\n  - on: "run.completed"\n    label: "build"\n    do: "ignore"\nbudget:\n  concurrent_runs: 4\n  total_runs: 25\n  reactions: 10')
    fireEvent.change(yaml, { target: { value: yaml.value.replace('total_runs: 25', 'total_runs: 40') } })
    fireEvent.click(editor().getByRole('button', { name: 'Save hooks and budget' }))
    await waitFor(() => expect(calls.at(-1)?.body).toEqual({ revision: 2, yaml: expect.stringContaining('total_runs: 40'), actor: 'human' }))
    fireEvent.click(editor().getByRole('button', { name: 'Pause' }))
    await waitFor(() => expect(editor().getByRole('button', { name: 'Resume' })).toBeInTheDocument())
    expect(calls.at(-1)?.url).toBe('/workspace/api/missions/task-1/pause?project_path=%2Fproject')
    expect(within(screen.getByRole('region', { name: 'In progress' })).getByTestId('mission-execution-chip')).toHaveTextContent('Running · Paused')
    fireEvent.click(editor().getByRole('button', { name: 'Open run build' }))
    expect(useStore.getState().viewMode).toBe('runs')
})

it('keeps unsaved hooks YAML across a live revision bump', async () => {
    task = started() as typeof task
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    const yaml = editor().getByLabelText('YAML') as HTMLTextAreaElement
    const edited = yaml.value.replace('total_runs: 25', 'total_runs: 40')
    fireEvent.change(yaml, { target: { value: edited } })
    await live('/project', { ...task, revision: 3, fields: { ...task.fields, budget: { concurrent_runs: 4, total_runs: 30, reactions: 10 } } })
    expect(editor().getByLabelText('YAML')).toHaveValue(edited)
    // Without edits the text follows the server.
    fireEvent.change(editor().getByLabelText('YAML'), { target: { value: hooksText(30) } })
    await live('/project', { ...task, revision: 4, fields: { ...task.fields, budget: { concurrent_runs: 4, total_runs: 35, reactions: 10 } } })
    expect(editor().getByLabelText('YAML')).toHaveValue(hooksText(35))
})
const hooksText = (total: number) => `hooks:\n  - on: "run.completed"\n    label: "build"\n    do: "ignore"\nbudget:\n  concurrent_runs: 4\n  total_runs: ${total}\n  reactions: 10`

it('shows a message sent while paused, fetching only newer events', async () => {
    task = { ...started(), paused: true, event_seq: 1 } as typeof task
    const events = [{ seq: 1, id: 'a', at: 't', kind: 'mission.started', source: 'human', payload: {} }]
    vi.mocked(fetch).mockImplementation(async (url, init) => {
        const text = String(url)
        if (init?.body) {
            calls.push({ url: text, body: JSON.parse(String(init.body)) })
            // Paused: the event is stored but the record is otherwise unchanged.
            events.push({ seq: 2, id: 'b', at: 't', kind: 'human.message', source: 'human', payload: { message: 'While paused' } })
            task = { ...task, event_seq: 2 } as typeof task
            return { ok: true, json: async () => task } as Response
        }
        if (text.includes('/events')) {
            const after = Number(new URL(text, 'http://x').searchParams.get('after') ?? 0)
            return { ok: true, json: async () => ({ events: events.filter(event => event.seq > after) }) } as Response
        }
        return { ok: true, json: async () => ({ missions: [task] }) } as Response
    })
    render(<MissionsPanel active />)
    fireEvent.click(await screen.findByRole('button', { name: 'Deliver search' }))
    expect(await editor().findByText('mission.started')).toBeInTheDocument()
    fireEvent.change(editor().getByLabelText('Message'), { target: { value: 'While paused' } })
    fireEvent.click(editor().getByRole('button', { name: 'Send message' }))
    expect(await editor().findByText('While paused')).toBeInTheDocument()
    expect(vi.mocked(fetch).mock.calls.some(([url]) => url === '/workspace/api/missions/task-1/events?after=1&project_path=%2Fproject')).toBe(true)
    expect(editor().getAllByText('mission.started')).toHaveLength(1)
})
