import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useStore } from '@/store'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { Input } from '@/components/ui/input'
import { RefreshCw } from 'lucide-react'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { MissionDetail } from './MissionDetail'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { MissionEditor } from './MissionEditor'

export type Budget = { concurrent_runs: number; total_runs: number }
export type Fields = { title: string; description: string; archived: boolean; budget?: Budget }
export type Status = 'draft' | 'running' | 'needs_you' | 'closed'
export type RosterEntry = { run_id: string; flow_name: string; summary: string; launched_at: string; status: string }
export type Mission = {
    id: string; revision: number; updated_at?: string; fields: Fields; activity: { revision: number; actor: string; at: string; note: string; before?: Fields; after?: Fields }[]
    status?: Status; conversation_id?: string | null; runs?: RosterEntry[]; cursor?: number; event_seq?: number
    closed?: { status: 'done' | 'failed' | 'canceled'; reason: string; at: string; actor?: string } | null; started_at?: string | null
}
type Draft = { editing: Mission | null; fields: Fields; conflict: boolean }
export type Board = { missions: Mission[] }
const empty: Fields = { title: '', description: '', archived: false }
export const groups: [Status, string][] = [['needs_you', 'Needs you'], ['running', 'Running'], ['draft', 'Drafts'], ['closed', 'Closed']]
export const statusLabels: Record<Status, string> = { needs_you: 'Needs you', running: 'Running', draft: 'Draft', closed: 'Closed' }
const terminal = ['completed', 'failed', 'canceled', 'validation_error']
/** One line saying why a mission is where it is. */
export function statusLine(mission: Mission): string {
    const runs = mission.runs ?? []
    const inFlight = runs.filter(run => !terminal.includes(run.status)).length
    const gate = runs.find(run => run.status === 'waiting')
    switch (mission.status ?? 'draft') {
        case 'draft': return 'Not started'
        case 'running': return inFlight ? `${inFlight} run${inFlight === 1 ? '' : 's'} in flight` : 'Agent is working'
        case 'needs_you': return gate ? `${gate.summary || gate.flow_name} is waiting on a human gate` : 'Waiting for your reply'
        case 'closed': return `Closed as ${mission.closed?.status ?? 'done'}${mission.closed?.reason ? `: ${mission.closed.reason}` : ''}`
    }
}
/** Mission timestamps are UTC `YYYY-MM-DD HH:MM:SS.fraction +00:00:00`. */
export function formatUpdated(value = ''): string {
    const match = /^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2})/.exec(value)
    const date = match ? new Date(`${match[1]}T${match[2]}Z`) : null
    return date && !Number.isNaN(date.getTime()) ? date.toLocaleString() : value
}
export class MissionConflict extends Error {}
/** Reads with no body; posts to `action` routes; creates without an id; otherwise patches. */
export async function request<T>(project: string, id = '', body?: unknown, action = ''): Promise<T> {
    const method = body === undefined ? undefined : action || !id ? 'POST' : 'PATCH'
    const response = await fetch(`/workspace/api/missions${id ? `/${encodeURIComponent(id)}` : ''}${action}${action.includes('?') ? '&' : '?'}project_path=${encodeURIComponent(project)}`, method ? { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : undefined)
    const value = await response.json()
    if (response.status === 409) throw new MissionConflict((action && value.detail) || 'This mission changed on the server. Your edits are preserved. Refresh and reconcile the latest revision before saving.')
    if (!response.ok) throw new Error(value.detail || 'Mission request failed')
    return value as T
}
export function MissionsPanel({ active }: { active: boolean }) {
    const project = useStore(s => s.activeProjectPath)
    const [projects, setProjects] = useState<string[]>([])
    // Keep each session's controller alive so pending saves retain their owner.
    if (project && !projects.includes(project)) setProjects([...projects, project])
    return <>{projects.map(path => <ProjectMissions key={path} project={path} selected={path === project} active={active && path === project} />)}
        {!project && <p>Select a project to manage missions.</p>}</>
}
function ProjectMissions({ project, selected, active }: { project: string; selected: boolean; active: boolean }) {
    const [board, setBoard] = useState<Board>({ missions: [] })
    const [editing, setEditing] = useState<Mission | null | undefined>(undefined)
    const [mode, setMode] = useState<'read' | 'edit'>('read')
    const [search, setSearch] = useState('')
    const [draft, setDraft] = useState<Fields>(empty)
    const [conflict, setConflict] = useState(false)
    const drafts = useRef(new Map<string, Draft>())
    const searchInput = useRef<HTMLInputElement>(null)
    const boardScroll = useRef<HTMLDivElement>(null)
    const scrollPositions = useRef(new Map<string, number>())
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
        const load = async () => { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (!disposed && sequence === refreshSequence.current) { setBoard(value); setLoaded(true); setLoadError('') } } catch (e) { if (!disposed && sequence === refreshSequence.current) setLoadError((e as Error).message) } }
        void load()
        // Live mission.upsert envelopes replace polling; a resync refetches the board.
        const onLive = (event: Event) => {
            const detail = (event as CustomEvent<{ projectPath?: string | null; mission?: Mission | null }>).detail
            if (detail?.projectPath && detail.projectPath !== project) return
            if (detail?.mission) upsert(detail.mission)
            else void load()
        }
        window.addEventListener('spark:mission-live-event', onLive)
        return () => { disposed = true; window.removeEventListener('spark:mission-live-event', onLive) }
    }, [project, active])
    function upsert(saved: Mission) {
        setBoard(current => ({ missions: current.missions.some(mission => mission.id === saved.id) ? current.missions.map(mission => mission.id === saved.id ? saved : mission) : [...current.missions, saved] }))
    }
    async function refresh() { const sequence = ++refreshSequence.current; try { const value = await request<Board>(project); if (sequence === refreshSequence.current) { setBoard(value); setLoaded(true); setLoadError('') } } catch (e) { if (sequence === refreshSequence.current) setLoadError((e as Error).message) } }
    function filter(nextSearch: string, nextArchived: boolean) {
        scrollPositions.current.set(JSON.stringify([search, archived]), boardScroll.current?.scrollTop ?? 0)
        setSearch(nextSearch); setArchived(nextArchived)
    }
    useLayoutEffect(() => {
        const position = scrollPositions.current.get(JSON.stringify([search, archived]))
        if (position !== undefined && boardScroll.current) boardScroll.current.scrollTop = position
    }, [search, archived])
    function preserveDraft() {
        if (mode !== 'edit' || editing === undefined) return
        const key = editing?.id ?? 'new'
        if (unsaved || conflict) drafts.current.set(key, { editing, fields: draft, conflict })
        else drafts.current.delete(key)
    }
    function open(mission: Mission | null, trigger: HTMLElement) {
        if (busy) return
        origin.current = trigger
        setFocusRequest(value => value + 1)
        preserveDraft()
        const cached = drafts.current.get(mission?.id ?? 'new')
        setMode(cached || !mission ? 'edit' : 'read')
        setEditing(cached?.editing ?? mission); setDraft(cached?.fields ?? mission?.fields ?? { ...empty }); setConflict(cached?.conflict ?? false); setError('')
    }
    function close(preserve = true) {
        if (busy) return
        if (preserve) preserveDraft()
        setEditing(undefined); setError('')
        window.setTimeout(() => {
            const target = origin.current?.isConnected ? origin.current : Array.from(boardScroll.current?.querySelectorAll<HTMLButtonElement>('[data-mission-id]') ?? []).find(card => card.dataset.missionId === origin.current?.dataset.missionId)
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
    function reset(mission: Mission | null) { setEditing(mission); setDraft(mission?.fields ?? { ...empty }); setConflict(false); setError(''); drafts.current.delete(mission?.id ?? 'new') }
    function reconcile() {
        if (!latest || !editing) return
        const local = Object.fromEntries(Object.entries(draft).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(editing.fields[key as keyof Fields])))
        setDraft({ ...latest.fields, ...local }); setEditing(latest); setConflict(false); setError('')
    }
    async function save(archive?: boolean) {
        if (busy || changed || conflict) return
        setBusy(true); setError('')
        try {
            const saved = await request<Mission>(project, editing?.id, { revision: editing?.revision, fields: archive === undefined ? draft : { archived: archive }, actor: 'human' })
            drafts.current.delete(editing?.id ?? 'new')
            upsert(saved)
            if (archive === undefined) { reset(saved); setMode('read') }
            else { setEditing(saved); setDraft(current => ({ ...current, archived: saved.fields.archived })) }
            await refresh()
        } catch (e) { setError((e as Error).message); if (e instanceof MissionConflict) { setConflict(true); await refresh() } } finally { setBusy(false) }
    }
    async function archive(mission: Mission, value: boolean) {
        if (busy) return
        setBusy(true); setError('')
        try {
            const saved = await request<Mission>(project, mission.id, { revision: mission.revision, fields: { archived: value }, actor: 'human' })
            upsert(saved)
            if (editing?.id === saved.id && mode === 'read') reset(saved)
        } catch (e) {
            setError(e instanceof MissionConflict ? 'This mission changed on the server. Refresh and try again.' : (e as Error).message)
            if (e instanceof MissionConflict) await refresh()
        } finally { setBusy(false) }
    }
    function edit() {
        reset(latest ?? editing ?? null)
        setMode('edit'); setFocusRequest(value => value + 1)
    }
    const latest = editing ? board.missions.find(t => t.id === editing.id) : undefined
    const changed = latest && latest.revision > (editing?.revision ?? 0)
    const unsaved = JSON.stringify(draft) !== JSON.stringify(editing?.fields ?? empty)
    const reading = mode === 'read' && Boolean(editing)
    const visible = board.missions.filter(mission => (archived || !mission.fields.archived) && mission.fields.title.toLowerCase().includes(search.toLowerCase()))
    if (!selected) return null
    return <section aria-label="Project missions" className="flex h-full min-h-0 flex-col gap-4 p-3 lg:p-6">
        <div className="flex shrink-0 flex-wrap items-center gap-2"><h1 ref={boardHeading} tabIndex={-1} className="text-xl font-semibold tracking-tight">Missions</h1>
            <Input ref={searchInput} type="search" placeholder="Search titles" aria-label="Search titles" value={search} onChange={e => filter(e.target.value, archived)} className="ml-auto h-8 w-56" />
            {search && <Button type="button" variant="ghost" size="sm" onClick={() => { filter('', archived); searchInput.current?.focus() }}>Clear search</Button>}
            <Button type="button" variant="outline" size="sm" className="aria-pressed:bg-accent aria-pressed:text-accent-foreground" aria-pressed={archived} onClick={() => filter(search, !archived)}>Show archived</Button>
            <TooltipProvider><Tooltip><TooltipTrigger asChild><Button type="button" variant="ghost" size="icon-sm" aria-label="Refresh" disabled={busy} onClick={() => void refresh()}><RefreshCw aria-hidden="true" className="size-4" /></Button></TooltipTrigger><TooltipContent>Refresh missions</TooltipContent></Tooltip></TooltipProvider>
            <Button type="button" size="sm" disabled={busy} onClick={e => open(null, e.currentTarget)}>Create mission</Button>
        </div>
        {(error || loadError) && editing === undefined && <InlineError>{error || loadError}</InlineError>}
        <div className="flex min-h-0 flex-1 gap-4">
        <div ref={boardScroll} hidden={narrow && editing !== undefined} className={`min-h-0 min-w-0 space-y-4 overflow-y-auto ${reading && !narrow ? 'w-80 shrink-0' : 'flex-1'}`}>
            {groups.map(([status, label]) => {
                const missions = visible.filter(mission => (mission.status ?? 'draft') === status).sort((a, b) => (b.updated_at ?? '').localeCompare(a.updated_at ?? ''))
                if (!missions.length) return null
                return <section key={status} aria-label={label} className="space-y-2">
                    <h2 className="flex items-center justify-between px-1 text-sm font-semibold">{label}<span aria-label={`${missions.length} matching missions`} className="text-xs font-normal text-muted-foreground">{missions.length}</span></h2>
                    <ul className="space-y-2">{missions.map(mission => <li key={mission.id} className="flex items-stretch gap-2">
                        <button type="button" disabled={busy} data-mission-id={mission.id} aria-label={mission.fields.title} aria-describedby={`mission-status-${mission.id}`} aria-pressed={editing?.id === mission.id} onClick={e => open(mission, e.currentTarget)} className={`block min-w-0 flex-1 rounded-md border p-3 text-left text-sm break-words transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50 ${editing?.id === mission.id ? 'border-ring bg-accent text-accent-foreground' : 'border-border bg-card text-card-foreground hover:border-foreground/40 hover:bg-muted'}`}>
                            <span className="line-clamp-2 font-semibold">{mission.fields.title}</span>
                            <span id={`mission-status-${mission.id}`} data-testid="mission-status-line" className={`mt-1 block text-xs ${status === 'needs_you' ? 'font-medium text-foreground' : 'text-muted-foreground'}`}>{statusLine(mission)}</span>
                            <span className="mt-1 block text-xs text-muted-foreground">Updated <time dateTime={mission.updated_at}>{formatUpdated(mission.updated_at)}</time></span>
                            {mission.fields.archived && <span className="mt-2 inline-block rounded border border-border px-1.5 py-0.5 text-xs text-muted-foreground">Archived</span>}
                        </button>
                        {status === 'closed' && <Button type="button" variant="outline" size="sm" className="h-auto self-stretch" disabled={busy} aria-label={`${mission.fields.archived ? 'Restore' : 'Archive'} ${mission.fields.title}`} onClick={() => void archive(mission, !mission.fields.archived)}>{mission.fields.archived ? 'Restore' : 'Archive'}</Button>}
                    </li>)}</ul>
                </section>
            })}
            {!visible.length && (loaded && !loadError
                ? <Empty className="px-3 py-4 text-xs text-muted-foreground"><EmptyDescription>{search ? 'No matches' : 'No missions'}</EmptyDescription></Empty>
                : <p className="px-1 py-2 text-sm text-muted-foreground">{loadError ? 'Unavailable' : 'Loading…'}</p>)}
        </div>
        {editing !== undefined && (mode === 'read' && editing ? <MissionDetail key={(latest ?? editing).id} mission={latest ?? editing} project={project} busy={busy} error={error || loadError} narrow={narrow} focusRequest={focusRequest} edit={edit} close={close} archive={value => archive(latest ?? editing, value)} onChange={upsert} /> : <MissionEditor key={editing?.id ?? 'new'} editing={editing} draft={draft} latest={latest} busy={busy} conflict={conflict} error={error || loadError} unsaved={unsaved} narrow={narrow} focusRequest={focusRequest}
            setDraft={setDraft} save={() => save()} archive={() => save(!editing?.fields.archived)} close={close} discard={discard} reconcile={reconcile} />)}
        </div>
    </section>
}
