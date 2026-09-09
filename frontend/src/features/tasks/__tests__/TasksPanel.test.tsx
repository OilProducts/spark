import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TasksPanel } from '../TasksPanel'
import { useStore } from '@/store'
import { selectSelectedRunId } from '@/state/runsSessionSelectors'

const fields = { title: 'Deliver search', description: '', acceptance_criteria: '', next_action: '', stage: 'ready', priority: 2, blocked: '', needs_input: '', archived: false, conversations: [], artifacts: [], runs: [{ run_id: 'run-1', stage: 'planning' }] }
let task: { id: string; revision: number; fields: typeof fields; activity: unknown[] }
let attention: { task_id: string; run_id: string }[]
let calls: { url: string; body: Record<string, unknown> }[]
beforeEach(() => {
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
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.click(screen.getByRole('button', { name: /Deliver search/ }))
    expect(screen.getByLabelText('Title')).toHaveValue('Unfinished edit')
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
    await act(async () => { useStore.setState({ activeProjectPath: '/other' }) })
    expect(screen.queryByLabelText('Title')).not.toBeInTheDocument()
    await act(async () => { useStore.setState({ activeProjectPath: '/project' }) })
    expect(screen.getByLabelText('Title')).toHaveValue('Project A draft')
})

it('creates a manual task and supports archival through accessible controls', async () => {
    render(<TasksPanel active />)
    await screen.findByRole('button', { name: /Deliver search/ })
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }))
    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'Manual outcome' } })
    fireEvent.change(screen.getByLabelText('Priority'), { target: { value: '1' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save task' }))
    expect(await screen.findByRole('button', { name: /Manual outcome.*High/ })).toBeInTheDocument()
    expect(task.fields.runs).toEqual([])
    expect(calls[0].body.revision).toBeUndefined()
    fireEvent.click(screen.getByLabelText('Archived'))
    fireEvent.click(screen.getByRole('button', { name: 'Save task' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: /Manual outcome.*High/ })).not.toBeInTheDocument())
    fireEvent.click(screen.getByLabelText('Show archived'))
    expect(screen.getByRole('button', { name: /Manual outcome.*High.*Archived/ })).toBeInTheDocument()
})
