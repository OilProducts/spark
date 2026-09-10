import { useEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { TaskEditor } from './TaskEditor'
import { buildRunsScopeKey } from '@/state/runsSessionScope'

export const stages = ['backlog', 'planning', 'ready', 'in_progress', 'review', 'done'] as const
export const labels = ['Backlog', 'Planning', 'Ready', 'In progress', 'Review', 'Done']
export type Stage = typeof stages[number]
export type Fields = {
    title: string; description: string; acceptance_criteria: string; next_action: string
    stage: Stage; priority: number; blocked: string; needs_input: string; archived: boolean
    conversations: string[]; artifacts: string[]; runs: { run_id: string; stage: Stage | null }[]
}
export type Task = { id: string; revision: number; fields: Fields; activity: { revision: number; actor: string; at: string; note: string; before?: Fields; after?: Fields; associated_run_id?: string; conversation_id?: string }[] }
type Draft = { editing: Task | null; fields: Fields; note: string; conflict: boolean }
export type Board = { tasks: Task[]; runs: { run_id: string; status: string }[]; attention: { task_id: string; run_id: string }[] }
const empty: Fields = { title: '', description: '', acceptance_criteria: '', next_action: '', stage: 'backlog', priority: 2, blocked: '', needs_input: '', archived: false, conversations: [], artifacts: [], runs: [] }
class TaskConflict extends Error {}
async function request<T>(project: string, id = '', body?: unknown): Promise<T> {
    const response = await fetch(`/workspace/api/tasks${id ? `/${encodeURIComponent(id)}` : ''}?project_path=${encodeURIComponent(project)}`, body ? { method: id ? 'PATCH' : 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : undefined)
    const value = await response.json()
    if (response.status === 409) throw new TaskConflict('This task changed on the server. Your edits are preserved. Refresh and reconcile the latest revision before saving.')
    if (!response.ok) throw new Error(value.detail || 'Task request failed')
    return value as T
}
export function TasksPanel({ active }: { active: boolean }) {
    const project = useStore(s => s.activeProjectPath)
    const [projects, setProjects] = useState<string[]>([])
    // Keep each session's controller alive so pending saves retain their owner.
    if (project && !projects.includes(project)) setProjects([...projects, project])
    return <>{projects.map(path => <ProjectTasks key={path} project={path} selected={path === project} active={active && path === project} />)}
        {!project && <p>Select a project to manage tasks.</p>}</>
}
function ProjectTasks({ project, selected, active }: { project: string; selected: boolean; active: boolean }) {
    const [board, setBoard] = useState<Board>({ tasks: [], runs: [], attention: [] })
    const [editing, setEditing] = useState<Task | null | undefined>(undefined)
    const [draft, setDraft] = useState<Fields>(empty)
    const [note, setNote] = useState('')
    const [conflict, setConflict] = useState(false)
    const drafts = useRef(new Map<string, Draft>())
    const boardHeading = useRef<HTMLHeadingElement>(null)
    const origin = useRef<HTMLElement | null>(null)
    const [focusRequest, setFocusRequest] = useState(0)
    const narrow = useNarrowViewport()
    const refreshSequence = useRef(0)
    const [error, setError] = useState('')
    const [busy, setBusy] = useState(false)
    const [attentionOnly, setAttentionOnly] = useState(false)
    const [archived, setArchived] = useState(false)
    const setView = useStore(s => s.setViewMode)
    const selectRun = useStore(s => s.setRunsSelectedRunIdForScope)
    const updateProject = useStore(s => s.updateProjectSessionState)
    useEffect(() => {
        if (!active) return
        let disposed = false
        const poll = async () => { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (!disposed && sequence === refreshSequence.current) setBoard(value) } catch (e) { if (!disposed && sequence === refreshSequence.current) setError((e as Error).message) } }
        void poll()
        const timer = window.setInterval(poll, 15000)
        return () => { disposed = true; window.clearInterval(timer) }
    }, [project, active])
    async function refresh() { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (sequence === refreshSequence.current) setBoard(value) } catch (e) { if (sequence === refreshSequence.current) setError((e as Error).message) } }
    function open(task: Task | null, trigger: HTMLElement) {
        if (busy) return
        origin.current = trigger
        setFocusRequest(value => value + 1)
        if (editing !== undefined) drafts.current.set(editing?.id ?? 'new', { editing, fields: draft, note, conflict })
        const cached = drafts.current.get(task?.id ?? 'new')
        setEditing(cached?.editing ?? task); setDraft(cached?.fields ?? task?.fields ?? { ...empty }); setNote(cached?.note ?? ''); setConflict(cached?.conflict ?? false); setError('')
    }
    function close(preserve = true) {
        if (busy) return
        if (preserve && editing !== undefined) drafts.current.set(editing?.id ?? 'new', { editing, fields: draft, note, conflict })
        setEditing(undefined); setError('')
        window.setTimeout(() => {
            const target = origin.current
            if (target?.isConnected && target.getClientRects().length) target.focus()
            else boardHeading.current?.focus()
        }, 0)
    }
    function discard() {
        if (editing) {
            reset(latest ?? editing)
            if (conflict && !changed) setConflict(true)
        }
        else { drafts.current.delete('new'); setDraft({ ...empty }); setNote(''); close(false) }
    }
    function reset(task: Task | null) { setEditing(task); setDraft(task?.fields ?? { ...empty }); setNote(''); setConflict(false); setError(''); drafts.current.delete(task?.id ?? 'new') }
    function reconcile() {
        if (!latest || !editing) return
        const local = Object.fromEntries(Object.entries(draft).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(editing.fields[key as keyof Fields])))
        setDraft({ ...latest.fields, ...local }); setEditing(latest); setConflict(false); setError('')
    }
    function run(id: string) { selectRun(buildRunsScopeKey(useStore.getState().runsListSession.scopeMode, project), id); setView('runs') }
    const needsAttention = (task: Task) => Boolean(task.fields.blocked || task.fields.needs_input || board.attention.some(a => a.task_id === task.id))
    async function save() {
        if (busy || changed || conflict) return
        setBusy(true); setError('')
        try {
            const saved = await request<Task>(project, editing?.id, { revision: editing?.revision, fields: { ...draft, conversations: draft.conversations.map(s => s.trim()).filter(Boolean), artifacts: draft.artifacts.map(s => s.trim()).filter(Boolean), runs: draft.runs.filter(r => r.run_id.trim()).map(r => ({ ...r, run_id: r.run_id.trim() })) }, note, actor: 'human' })
            drafts.current.delete(editing?.id ?? 'new')
            setBoard(current => ({ ...current, tasks: [...current.tasks.filter(task => task.id !== saved.id), saved] }))
            reset(saved)
            await refresh()
        } catch (e) { setError((e as Error).message); if (e instanceof TaskConflict) { setConflict(true); await refresh() } } finally { setBusy(false) }
    }
    const latest = editing ? board.tasks.find(t => t.id === editing.id) : undefined
    const changed = latest && latest.revision > (editing?.revision ?? 0)
    const unsaved = JSON.stringify(draft) !== JSON.stringify(editing?.fields ?? empty) || note !== ''
    if (!selected) return null
    return <section aria-label="Project tasks" className="flex h-full min-h-0 flex-col gap-4 p-4">
        <div className="flex shrink-0 flex-wrap gap-4 items-center"><h1 ref={boardHeading} tabIndex={-1} className="text-xl font-semibold">Tasks</h1><Button type="button" disabled={busy} onClick={e => open(null, e.currentTarget)}>Create task</Button><Button type="button" variant="secondary" disabled={busy} onClick={() => void refresh()}>Refresh</Button>
            <Label><Checkbox checked={attentionOnly} onCheckedChange={value => setAttentionOnly(value === true)} /> Needs attention</Label>
            <Label><Checkbox checked={archived} onCheckedChange={value => setArchived(value === true)} /> Show archived</Label>
        </div>
        {error && editing === undefined && <p role="alert">{error}</p>}
        <div className="flex min-h-0 flex-1 gap-4">
        <div hidden={narrow && editing !== undefined} className="min-w-0 flex-1 overflow-auto">
        <div className="grid grid-cols-[repeat(auto-fit,minmax(10rem,1fr))] items-start gap-3">
            {stages.map((stage, i) => <section key={stage} aria-label={labels[i]} className="rounded border p-2 space-y-2"><h2 className="font-semibold">{labels[i]}</h2>
                {board.tasks.filter(t => t.fields.stage === stage && (archived || !t.fields.archived) && (!attentionOnly || needsAttention(t))).map(task => <button type="button" disabled={busy} key={task.id} onClick={e => open(task, e.currentTarget)} className="block w-full rounded border bg-background p-3 text-left focus-visible:outline-2 focus-visible:outline-ring">
                    <strong>{task.fields.title}</strong><span className="block">{['Urgent', 'High', 'Normal', 'Low'][task.fields.priority]}</span>
                    {task.fields.blocked && <span className="block">Blocked: {task.fields.blocked}</span>}{task.fields.needs_input && <span className="block">Needs input: {task.fields.needs_input}</span>}
                    {board.attention.some(a => a.task_id === task.id) && <span className="block">Run needs input</span>}{task.fields.archived && <span>Archived</span>}
                </button>)}
            </section>)}
        </div></div>
        {editing !== undefined && <TaskEditor key={editing?.id ?? 'new'} editing={editing} draft={draft} note={note} board={board} latest={latest} busy={busy} conflict={conflict} error={error} unsaved={unsaved} narrow={narrow} focusRequest={focusRequest}
            setDraft={setDraft} setNote={setNote} save={save} close={close} discard={discard} reconcile={reconcile} run={run}
            conversation={id => { updateProject(project, { conversationId: id }); setView('home') }} />}
        </div>
    </section>
}
