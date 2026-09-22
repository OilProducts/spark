import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { Input } from '@/components/ui/input'
import { RefreshCw } from 'lucide-react'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { TaskDetail } from './TaskDetail'
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
    const [mode, setMode] = useState<'read' | 'edit'>('read')
    const [search, setSearch] = useState('')
    const [draft, setDraft] = useState<Fields>(empty)
    const [conflict, setConflict] = useState(false)
    const drafts = useRef(new Map<string, Draft>())
    const searchInput = useRef<HTMLInputElement>(null)
    const boardScroll = useRef<HTMLDivElement>(null)
    const laneScroll = useRef<(HTMLDivElement | null)[]>([])
    const scrollPositions = useRef(new Map<string, number[]>())
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
    function filter(nextSearch: string, nextArchived: boolean) {
        scrollPositions.current.set(JSON.stringify([search, archived]), [boardScroll.current?.scrollLeft ?? 0, ...laneScroll.current.map(lane => lane?.scrollTop ?? 0)])
        setSearch(nextSearch); setArchived(nextArchived)
    }
    useLayoutEffect(() => {
        const position = scrollPositions.current.get(JSON.stringify([search, archived]))
        if (!position) return
        if (boardScroll.current) boardScroll.current.scrollLeft = position[0]
        laneScroll.current.forEach((lane, i) => { if (lane) lane.scrollTop = position[i + 1] })
    }, [search, archived])
    function preserveDraft() {
        if (mode !== 'edit' || editing === undefined) return
        const key = editing?.id ?? 'new'
        if (unsaved || conflict) drafts.current.set(key, { editing, fields: draft, conflict })
        else drafts.current.delete(key)
    }
    function open(task: Task | null, trigger: HTMLElement) {
        if (busy) return
        origin.current = trigger
        setFocusRequest(value => value + 1)
        preserveDraft()
        const cached = drafts.current.get(task?.id ?? 'new')
        setMode(cached || !task ? 'edit' : 'read')
        setEditing(cached?.editing ?? task); setDraft(cached?.fields ?? task?.fields ?? { ...empty }); setConflict(cached?.conflict ?? false); setError('')
    }
    function close(preserve = true) {
        if (busy) return
        if (preserve) preserveDraft()
        setEditing(undefined); setError('')
        window.setTimeout(() => {
            const target = origin.current?.isConnected ? origin.current : Array.from(boardScroll.current?.querySelectorAll<HTMLButtonElement>('[data-task-id]') ?? []).find(card => card.dataset.taskId === origin.current?.dataset.taskId)
            if (target?.isConnected && target.getClientRects().length) target.focus({ preventScroll: true })
            else boardHeading.current?.focus({ preventScroll: true })
        }, 0)
    }
    function discard() {
        if (editing) {
            reset(latest ?? editing)
            setMode('read')
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
            setBoard(current => ({ ...current, tasks: current.tasks.some(task => task.id === saved.id) ? current.tasks.map(task => task.id === saved.id ? saved : task) : [...current.tasks, saved] }))
            if (archive === undefined) { reset(saved); setMode('read') }
            else { setEditing(saved); setDraft(current => ({ ...current, archived: saved.fields.archived })) }
            await refresh()
        } catch (e) { setError((e as Error).message); if (e instanceof TaskConflict) { setConflict(true); await refresh() } } finally { setBusy(false) }
    }
    async function changeStage(stage: Stage) {
        const task = latest ?? editing
        if (busy || mode !== 'read' || !task) return
        setBusy(true); setError('')
        try {
            const saved = await request<Task>(project, task.id, { revision: task.revision, fields: { stage }, actor: 'human' })
            setBoard(current => ({ tasks: current.tasks.map(record => record.id === saved.id ? saved : record) }))
            reset(saved)
            await refresh()
        } catch (e) {
            setError(e instanceof TaskConflict ? 'This task changed on the server. Refresh the latest version and retry your stage change.' : (e as Error).message)
            if (e instanceof TaskConflict) await refresh()
        } finally { setBusy(false) }
    }
    function edit() {
        reset(latest ?? editing ?? null)
        setMode('edit'); setFocusRequest(value => value + 1)
    }
    const latest = editing ? board.tasks.find(t => t.id === editing.id) : undefined
    const changed = latest && latest.revision > (editing?.revision ?? 0)
    const unsaved = JSON.stringify(draft) !== JSON.stringify(editing?.fields ?? empty)
    if (!selected) return null
    return <section aria-label="Project tasks" className="flex h-full min-h-0 flex-col gap-4 p-3 lg:p-6">
        <div className="flex shrink-0 flex-wrap items-center gap-2"><h1 ref={boardHeading} tabIndex={-1} className="text-xl font-semibold tracking-tight">Tasks</h1>
            <Input ref={searchInput} type="search" placeholder="Search titles" aria-label="Search titles" value={search} onChange={e => filter(e.target.value, archived)} className="ml-auto h-8 w-56" />
            {search && <Button type="button" variant="ghost" size="sm" onClick={() => { filter('', archived); searchInput.current?.focus() }}>Clear search</Button>}
            <Button type="button" variant="outline" size="sm" className="aria-pressed:bg-accent aria-pressed:text-accent-foreground" aria-pressed={archived} onClick={() => filter(search, !archived)}>Show archived</Button>
            <TooltipProvider><Tooltip><TooltipTrigger asChild><Button type="button" variant="ghost" size="icon-sm" aria-label="Refresh" disabled={busy} onClick={() => void refresh()}><RefreshCw aria-hidden="true" className="size-4" /></Button></TooltipTrigger><TooltipContent>Refresh tasks</TooltipContent></Tooltip></TooltipProvider>
            <Button type="button" size="sm" disabled={busy} onClick={e => open(null, e.currentTarget)}>Create task</Button>
        </div>
        {(error || loadError) && editing === undefined && <InlineError>{error || loadError}</InlineError>}
        <div className="flex min-h-0 flex-1 gap-4">
        <div ref={boardScroll} hidden={narrow && editing !== undefined} className="min-w-0 flex-1 overflow-x-auto">
        <div className="grid h-full min-h-0 grid-cols-[repeat(6,minmax(11rem,1fr))] gap-3">
            {stages.map((stage, i) => {
                const tasks = board.tasks.filter(t => t.fields.stage === stage && (archived || !t.fields.archived) && t.fields.title.toLowerCase().includes(search.toLowerCase()))
                return <section key={stage} aria-label={labels[i]} className="flex min-h-0 flex-col rounded-md border border-border bg-muted/50 p-2">
                    <h2 className="flex shrink-0 items-center justify-between px-1 pb-3 pt-1 text-sm font-semibold">{labels[i]}<span aria-label={`${tasks.length} matching tasks`} className="text-xs font-normal text-muted-foreground">{tasks.length}</span></h2>
                    <div ref={node => { laneScroll.current[i] = node }} className="min-h-0 flex-1 space-y-2 overflow-y-auto p-1">
                    {tasks.map(task => <button type="button" disabled={busy} key={task.id} data-task-id={task.id} aria-label={task.fields.title} aria-pressed={editing?.id === task.id} onClick={e => open(task, e.currentTarget)} className={`block w-full rounded-md border p-3 text-left text-sm break-words transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 ${editing?.id === task.id ? 'border-ring bg-accent text-accent-foreground' : 'border-border bg-card text-card-foreground hover:border-foreground/40 hover:bg-muted'}`}>
                        <span className="line-clamp-3 font-semibold">{task.fields.title}</span>
                        {task.fields.description && <span className="mt-1 line-clamp-2 text-xs text-muted-foreground">{task.fields.description}</span>}
                        {task.fields.archived && <span className="mt-2 inline-block rounded border border-border px-1.5 py-0.5 text-xs text-muted-foreground">Archived</span>}
                    </button>)}
                    {!tasks.length && (loaded && !loadError
                        ? <Empty className="px-3 py-4 text-xs text-muted-foreground"><EmptyDescription>{search ? 'No matches' : 'No tasks'}</EmptyDescription></Empty>
                        : <p className="px-1 py-2 text-sm text-muted-foreground">{loadError ? 'Unavailable' : 'Loading…'}</p>)}
                    </div>
                </section>
            })}
        </div></div>
        {editing !== undefined && (mode === 'read' && editing ? <TaskDetail task={latest ?? editing} busy={busy} error={error || loadError} narrow={narrow} focusRequest={focusRequest} edit={edit} close={close} changeStage={changeStage} /> : <TaskEditor key={editing?.id ?? 'new'} editing={editing} draft={draft} latest={latest} busy={busy} conflict={conflict} error={error || loadError} unsaved={unsaved} narrow={narrow} focusRequest={focusRequest}
            setDraft={setDraft} save={() => save()} archive={() => save(!editing?.fields.archived)} close={close} discard={discard} reconcile={reconcile} />)}
        </div>
    </section>
}
