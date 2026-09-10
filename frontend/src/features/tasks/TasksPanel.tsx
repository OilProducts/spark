import { useEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { TaskEditor } from './TaskEditor'

export const stages = ['backlog', 'planning', 'ready', 'in_progress', 'review', 'done'] as const
export const labels = ['Backlog', 'Planning', 'Ready', 'In progress', 'Review', 'Done']
export type Stage = typeof stages[number]
export type Fields = { title: string; description: string; stage: Stage; archived: boolean }
export type Task = { id: string; revision: number; fields: Fields; activity: { revision: number; actor: string; at: string; note: string; before?: Fields; after?: Fields }[] }
type Draft = { editing: Task | null; fields: Fields; conflict: boolean }
export type Board = { tasks: Task[] }
const empty: Fields = { title: '', description: '', stage: 'backlog', archived: false }
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
    const [board, setBoard] = useState<Board>({ tasks: [] })
    const [editing, setEditing] = useState<Task | null | undefined>(undefined)
    const [draft, setDraft] = useState<Fields>(empty)
    const [conflict, setConflict] = useState(false)
    const drafts = useRef(new Map<string, Draft>())
    const boardHeading = useRef<HTMLHeadingElement>(null)
    const origin = useRef<HTMLElement | null>(null)
    const [focusRequest, setFocusRequest] = useState(0)
    const narrow = useNarrowViewport()
    const refreshSequence = useRef(0)
    const [loaded, setLoaded] = useState(false)
    const [loadError, setLoadError] = useState('')
    const [error, setError] = useState('')
    const [busy, setBusy] = useState(false)
    const [archived, setArchived] = useState(false)
    useEffect(() => {
        if (!active) return
        let disposed = false
        const poll = async () => { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (!disposed && sequence === refreshSequence.current) { setBoard(value); setLoaded(true); setLoadError('') } } catch (e) { if (!disposed && sequence === refreshSequence.current) setLoadError((e as Error).message) } }
        void poll()
        const timer = window.setInterval(poll, 15000)
        return () => { disposed = true; window.clearInterval(timer) }
    }, [project, active])
    async function refresh() { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (sequence === refreshSequence.current) { setBoard(value); setLoaded(true); setLoadError('') } } catch (e) { if (sequence === refreshSequence.current) setLoadError((e as Error).message) } }
    function open(task: Task | null, trigger: HTMLElement) {
        if (busy) return
        origin.current = trigger
        setFocusRequest(value => value + 1)
        if (editing !== undefined) drafts.current.set(editing?.id ?? 'new', { editing, fields: draft, conflict })
        const cached = drafts.current.get(task?.id ?? 'new')
        setEditing(cached?.editing ?? task); setDraft(cached?.fields ?? task?.fields ?? { ...empty }); setConflict(cached?.conflict ?? false); setError('')
    }
    function close(preserve = true) {
        if (busy) return
        if (preserve && editing !== undefined) drafts.current.set(editing?.id ?? 'new', { editing, fields: draft, conflict })
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
        else { drafts.current.delete('new'); setDraft({ ...empty }); close(false) }
    }
    function reset(task: Task | null) { setEditing(task); setDraft(task?.fields ?? { ...empty }); setConflict(false); setError(''); drafts.current.delete(task?.id ?? 'new') }
    function reconcile() {
        if (!latest || !editing) return
        const local = Object.fromEntries(Object.entries(draft).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(editing.fields[key as keyof Fields])))
        setDraft({ ...latest.fields, ...local }); setEditing(latest); setConflict(false); setError('')
    }
    async function save(archive?: boolean) {
        if (busy || changed || conflict) return
        setBusy(true); setError('')
        try {
            const saved = await request<Task>(project, editing?.id, { revision: editing?.revision, fields: archive === undefined ? draft : { archived: archive }, actor: 'human' })
            drafts.current.delete(editing?.id ?? 'new')
            setBoard(current => ({ ...current, tasks: [...current.tasks.filter(task => task.id !== saved.id), saved] }))
            if (archive === undefined) reset(saved)
            else { setEditing(saved); setDraft(current => ({ ...current, archived: saved.fields.archived })) }
            await refresh()
        } catch (e) { setError((e as Error).message); if (e instanceof TaskConflict) { setConflict(true); await refresh() } } finally { setBusy(false) }
    }
    const latest = editing ? board.tasks.find(t => t.id === editing.id) : undefined
    const changed = latest && latest.revision > (editing?.revision ?? 0)
    const unsaved = JSON.stringify(draft) !== JSON.stringify(editing?.fields ?? empty)
    if (!selected) return null
    return <section aria-label="Project tasks" className="flex h-full min-h-0 flex-col gap-4 p-3 lg:p-6">
        <div className="flex shrink-0 flex-wrap gap-2 items-center"><h1 ref={boardHeading} tabIndex={-1} className="text-lg font-semibold tracking-tight">Tasks</h1><Button type="button" disabled={busy} onClick={e => open(null, e.currentTarget)}>Create task</Button><Button type="button" variant="secondary" disabled={busy} onClick={() => void refresh()}>Refresh</Button>
            <Label><Checkbox checked={archived} onCheckedChange={value => setArchived(value === true)} /> Show archived</Label>
        </div>
        {(error || loadError) && editing === undefined && <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-foreground">{error || loadError}</p>}
        <div className="flex min-h-0 flex-1 gap-4">
        <div hidden={narrow && editing !== undefined} className="min-w-0 flex-1 overflow-auto">
        <div className="grid grid-cols-[repeat(6,minmax(10rem,1fr))] items-start gap-3">
            {stages.map((stage, i) => {
                const tasks = board.tasks.filter(t => t.fields.stage === stage && (archived || !t.fields.archived))
                return <section key={stage} aria-label={labels[i]} className="rounded-md border border-border bg-muted/20 p-2 space-y-2">
                    <h2 className="px-1 py-1 text-sm font-semibold">{labels[i]}</h2>
                    {tasks.map(task => <button type="button" disabled={busy} key={task.id} aria-pressed={editing?.id === task.id} onClick={e => open(task, e.currentTarget)} className={`block w-full rounded-md border p-3 text-left text-sm break-words transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 ${editing?.id === task.id ? 'border-ring bg-accent text-accent-foreground' : 'border-border bg-card text-card-foreground'}`}>
                        {task.fields.title}
                    </button>)}
                    {!tasks.length && <p className="px-1 py-2 text-sm text-muted-foreground">{loadError ? 'Unavailable' : loaded ? 'No tasks' : 'Loading…'}</p>}
                </section>
            })}
        </div></div>
        {editing !== undefined && <TaskEditor key={editing?.id ?? 'new'} editing={editing} draft={draft} latest={latest} busy={busy} conflict={conflict} error={error || loadError} unsaved={unsaved} narrow={narrow} focusRequest={focusRequest}
            setDraft={setDraft} save={() => save()} archive={() => save(!editing?.fields.archived)} close={close} discard={discard} reconcile={reconcile} />}
        </div>
    </section>
}
