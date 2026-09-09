import { useEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { buildRunsScopeKey } from '@/state/runsSessionScope'

const stages = ['backlog', 'planning', 'ready', 'in_progress', 'review', 'done'] as const
const labels = ['Backlog', 'Planning', 'Ready', 'In progress', 'Review', 'Done']
type Stage = typeof stages[number]
type Fields = {
    title: string; description: string; acceptance_criteria: string; next_action: string
    stage: Stage; priority: number; blocked: string; needs_input: string; archived: boolean
    conversations: string[]; artifacts: string[]; runs: { run_id: string; stage: Stage | null }[]
}
type Task = { id: string; revision: number; fields: Fields; activity: { revision: number; actor: string; at: string; note: string; before?: Fields; after?: Fields; associated_run_id?: string; conversation_id?: string }[] }
type Draft = { editing: Task | null; fields: Fields; note: string }
type Session = { editing: Task | null | undefined; draft: Fields; note: string; drafts: Map<string, Draft> }
type Board = { tasks: Task[]; runs: { run_id: string; status: string }[]; attention: { task_id: string; run_id: string }[] }
const empty: Fields = { title: '', description: '', acceptance_criteria: '', next_action: '', stage: 'backlog', priority: 2, blocked: '', needs_input: '', archived: false, conversations: [], artifacts: [], runs: [] }
async function request<T>(project: string, id = '', body?: unknown): Promise<T> {
    const response = await fetch(`/workspace/api/tasks${id ? `/${encodeURIComponent(id)}` : ''}?project_path=${encodeURIComponent(project)}`, body ? { method: id ? 'PATCH' : 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : undefined)
    const value = await response.json()
    if (!response.ok) throw new Error(response.status === 409 ? 'This task changed on the server. Your edits are preserved. Reload the latest task and reconcile before saving.' : value.detail || 'Task request failed')
    return value as T
}
export function TasksPanel({ active }: { active: boolean }) {
    const project = useStore(s => s.activeProjectPath)
    const sessions = useRef(new Map<string, Session>())
    return project ? <ProjectTasks key={project} project={project} active={active} sessions={sessions.current} /> : <p>Select a project to manage tasks.</p>
}
function ProjectTasks({ project, active, sessions }: { project: string; active: boolean; sessions: Map<string, Session> }) {
    const cached = sessions.get(project)
    const [board, setBoard] = useState<Board>({ tasks: [], runs: [], attention: [] })
    const [editing, setEditing] = useState<Task | null | undefined>(cached?.editing)
    const [draft, setDraft] = useState<Fields>(cached?.draft ?? empty)
    const [note, setNote] = useState(cached?.note ?? '')
    const drafts = useRef(cached?.drafts ?? new Map<string, Draft>())
    const heading = useRef<HTMLHeadingElement>(null)
    const refreshSequence = useRef(0)
    const [error, setError] = useState('')
    const [busy, setBusy] = useState(false)
    const [attentionOnly, setAttentionOnly] = useState(false)
    const [archived, setArchived] = useState(false)
    const setView = useStore(s => s.setViewMode)
    const selectRun = useStore(s => s.setRunsSelectedRunIdForScope)
    const updateProject = useStore(s => s.updateProjectSessionState)
    useEffect(() => { sessions.set(project, { editing, draft, note, drafts: drafts.current }) }, [project, sessions, editing, draft, note])
    useEffect(() => {
        if (!active) return
        let disposed = false
        const poll = async () => { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (!disposed && sequence === refreshSequence.current) setBoard(value) } catch (e) { if (!disposed && sequence === refreshSequence.current) setError((e as Error).message) } }
        void poll()
        const timer = window.setInterval(poll, 15000)
        return () => { disposed = true; window.clearInterval(timer) }
    }, [project, active])
    async function refresh() { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (sequence === refreshSequence.current) setBoard(value) } catch (e) { if (sequence === refreshSequence.current) setError((e as Error).message) } }
    function open(task: Task | null) {
        if (editing !== undefined) drafts.current.set(editing?.id ?? 'new', { editing, fields: draft, note })
        const cached = drafts.current.get(task?.id ?? 'new')
        setEditing(cached?.editing ?? task); setDraft(cached?.fields ?? task?.fields ?? { ...empty }); setNote(cached?.note ?? ''); setError('')
        window.setTimeout(() => heading.current?.focus(), 0)
    }
    function reset(task: Task | null) { setEditing(task); setDraft(task?.fields ?? { ...empty }); setNote(''); setError(''); drafts.current.delete(task?.id ?? 'new') }
    function reconcile() {
        if (!latest || !editing) return
        const local = Object.fromEntries(Object.entries(draft).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(editing.fields[key as keyof Fields])))
        setDraft({ ...latest.fields, ...local }); setEditing(latest); setError('')
    }
    function run(id: string) { selectRun(buildRunsScopeKey(useStore.getState().runsListSession.scopeMode, project), id); setView('runs') }
    const needsAttention = (task: Task) => Boolean(task.fields.blocked || task.fields.needs_input || board.attention.some(a => a.task_id === task.id))
    async function save() {
        setBusy(true); setError('')
        try {
            const saved = await request<Task>(project, editing?.id, { revision: editing?.revision, fields: { ...draft, conversations: draft.conversations.map(s => s.trim()).filter(Boolean), artifacts: draft.artifacts.map(s => s.trim()).filter(Boolean), runs: draft.runs.filter(r => r.run_id.trim()).map(r => ({ ...r, run_id: r.run_id.trim() })) }, note, actor: 'human' })
            drafts.current.delete(editing?.id ?? 'new')
            reset(saved)
            await refresh()
        } catch (e) { setError((e as Error).message) } finally { setBusy(false) }
    }
    const latest = editing ? board.tasks.find(t => t.id === editing.id) : undefined
    const changed = latest && latest.revision !== editing?.revision
    const textField = (key: 'title' | 'description' | 'acceptance_criteria' | 'next_action' | 'blocked' | 'needs_input', label: string) => <label className="grid gap-1" key={key}>{label}<textarea className="rounded border bg-background p-2" required={key === 'title'} value={draft[key]} onChange={e => setDraft({ ...draft, [key]: e.target.value })} /></label>
    return <section aria-label="Project tasks" className="h-full overflow-auto p-4 space-y-4">
        <div className="flex flex-wrap gap-4 items-center"><h1 className="text-xl font-semibold">Tasks</h1><button type="button" disabled={busy} onClick={() => open(null)}>Create task</button><button type="button" onClick={() => void refresh()}>Refresh</button>
            <label><input type="checkbox" checked={attentionOnly} onChange={e => setAttentionOnly(e.target.checked)} /> Needs attention</label>
            <label><input type="checkbox" checked={archived} onChange={e => setArchived(e.target.checked)} /> Show archived</label>
        </div>
        {error && <p role="alert">{error}</p>}
        <div className="grid grid-cols-1 md:grid-cols-3 xl:grid-cols-6 gap-3">
            {stages.map((stage, i) => <section key={stage} aria-label={labels[i]} className="rounded border p-2 space-y-2"><h2 className="font-semibold">{labels[i]}</h2>
                {board.tasks.filter(t => t.fields.stage === stage && (archived || !t.fields.archived) && (!attentionOnly || needsAttention(t))).map(task => <button type="button" disabled={busy} key={task.id} onClick={() => open(task)} className="block w-full rounded border bg-background p-3 text-left">
                    <strong>{task.fields.title}</strong><span className="block">{['Urgent', 'High', 'Normal', 'Low'][task.fields.priority]}</span>
                    {task.fields.blocked && <span className="block">Blocked: {task.fields.blocked}</span>}{task.fields.needs_input && <span className="block">Needs input: {task.fields.needs_input}</span>}
                    {board.attention.some(a => a.task_id === task.id) && <span className="block">Run needs input</span>}{task.fields.archived && <span>Archived</span>}
                </button>)}
            </section>)}
        </div>
        {editing !== undefined && <section aria-label="Task details" className="rounded border bg-background p-4 space-y-3">
            <h2 ref={heading} tabIndex={-1} className="font-semibold">{editing ? `Task ${editing.id} · revision ${editing.revision}` : 'New task'}</h2>
            {changed && <div role="status">A newer revision is available. Your draft is preserved. Review the server changes below before reconciling; your edited fields take precedence. <button type="button" disabled={busy} onClick={reconcile}>Reconcile with latest revision</button><pre className="whitespace-pre-wrap">{JSON.stringify(latest.fields, null, 2)}</pre></div>}
            <button type="button" disabled={busy} onClick={() => { if (latest) reset(latest); else reset(null) }}>Discard edits and reload</button>
            <form onSubmit={e => { e.preventDefault(); void save() }}><fieldset disabled={busy} className="grid gap-3">
                {textField('title', 'Title')}{textField('description', 'Outcome and decisions')}{textField('acceptance_criteria', 'Acceptance criteria')}{textField('next_action', 'Next action')}
                <label>Stage <select className="border bg-background p-2" value={draft.stage} onChange={e => setDraft({ ...draft, stage: e.target.value as Stage })}>{stages.map((s, i) => <option key={s} value={s}>{labels[i]}</option>)}</select></label>
                <label>Priority <select className="border bg-background p-2" value={draft.priority} onChange={e => setDraft({ ...draft, priority: Number(e.target.value) })}>{['Urgent', 'High', 'Normal', 'Low'].map((p, i) => <option key={p} value={i}>{p}</option>)}</select></label>
                {textField('blocked', 'Blocked — explanation (empty to clear)')}{textField('needs_input', 'Needs input — question (empty to clear)')}
                <label><input type="checkbox" checked={draft.archived} onChange={e => setDraft({ ...draft, archived: e.target.checked })} /> Archived</label>
                <label className="grid">Conversation IDs (one per line)<textarea className="border bg-background p-2" value={draft.conversations.join('\n')} onChange={e => setDraft({ ...draft, conversations: e.target.value.split('\n') })} /></label>
                <label className="grid">Repository artifact paths (one per line)<textarea className="border bg-background p-2" value={draft.artifacts.join('\n')} onChange={e => setDraft({ ...draft, artifacts: e.target.value.split('\n') })} /></label>
                <label className="grid">Run IDs (one per line)<textarea className="border bg-background p-2" value={draft.runs.map(r => r.run_id).join('\n')} onChange={e => setDraft({ ...draft, runs: e.target.value.split('\n').map(id => draft.runs.find(r => r.run_id === id) ?? { run_id: id, stage: null }) })} /></label>
                {draft.runs.map((r, i) => r.run_id && <label key={i}>Stage supported by {r.run_id} <select aria-label={`Stage supported by ${r.run_id}`} value={r.stage ?? ''} onChange={e => setDraft({ ...draft, runs: draft.runs.map((link, index) => index === i ? { ...link, stage: e.target.value ? e.target.value as Stage : null } : link) })}><option value="">Unspecified</option>{stages.map((s, index) => <option key={s} value={s}>{labels[index]}</option>)}</select></label>)}
                <label className="grid">Note / completion evidence<textarea className="border bg-background p-2" value={note} required={draft.stage === 'done' && editing?.fields.stage !== 'done'} onChange={e => setNote(e.target.value)} /></label>
                <button type="submit" disabled={busy || Boolean(changed)}>{busy ? 'Saving…' : 'Save task'}</button>
            </fieldset></form>
            <h3>Related resources</h3>
            {editing?.fields.runs.map(r => <button className="block" key={r.run_id} type="button" onClick={() => run(r.run_id)}>Open run {r.run_id} — {board.runs.find(run => run.run_id === r.run_id)?.status ?? 'Unavailable'}</button>)}
            {board.attention.filter(a => a.task_id === editing?.id).map(a => <button className="block" key={a.run_id} type="button" onClick={() => run(a.run_id)}>Answer questions in run {a.run_id}</button>)}
            {editing?.fields.conversations.map(id => <button className="block" key={id} type="button" onClick={() => { updateProject(project, { conversationId: id }); setView('home') }}>Open conversation {id}</button>)}
            {editing?.fields.artifacts.map(path => <p key={path}>Repository artifact: <code>{path}</code></p>)}
            <h3>Activity</h3><ol>{(latest ?? editing)?.activity.map(a => <li key={a.revision} className="border-b py-2">Revision {a.revision} · {a.actor} · {a.at}{a.conversation_id && ` · conversation ${a.conversation_id}`}<p>{a.note}</p>{a.associated_run_id && <p>Linked run {a.associated_run_id}</p>}{a.after && <details><summary>Changes</summary><pre className="whitespace-pre-wrap">{JSON.stringify(Object.fromEntries(Object.entries(a.after).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(a.before?.[key as keyof Fields]))), null, 2)}</pre></details>}</li>)}</ol>
        </section>}
    </section>
}
