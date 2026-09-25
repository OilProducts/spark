import { useEffect, useState } from 'react'
import { useStore } from '@/store'
import { buildRunsScopeKey } from '@/state/runsSessionScope'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { MissionConflict, request, substateLabels, type Mission, type MissionEvent } from './MissionsPanel'

/** Emits JSON-compatible YAML: block structure with JSON-quoted scalars. */
export function toYaml(value: unknown, indent = ''): string {
    if (Array.isArray(value)) return value.length ? value.map(item => `\n${indent}- ${toYaml(item, `${indent}  `).trimStart()}`).join('') : ' []'
    if (value && typeof value === 'object') {
        const entries = Object.entries(value)
        return entries.length ? entries.map(([key, item]) => `\n${indent}${key}:${toYaml(item, `${indent}  `)}`).join('') : ' {}'
    }
    return ` ${JSON.stringify(value)}`
}
const hooksYaml = (mission: Mission) => toYaml({ hooks: mission.fields.hooks ?? [], budget: mission.fields.budget ?? {} }).trimStart()
const describe = (event: MissionEvent) => {
    const payload = event.payload as { message?: unknown; status?: unknown } | null
    return typeof payload?.message === 'string' ? payload.message : typeof payload?.status === 'string' ? payload.status : ''
}

type Props = { mission: Mission; project: string; busy: boolean; onChange: (mission: Mission) => void }
export function MissionExecution({ mission, project, busy, onChange }: Props) {
    const [events, setEvents] = useState<MissionEvent[]>([])
    const [message, setMessage] = useState('')
    const [yaml, setYaml] = useState(() => hooksYaml(mission))
    const [pending, setPending] = useState(false)
    const [error, setError] = useState('')
    const started = Boolean(mission.started_at)
    const closed = Boolean(mission.closed)
    const savedYaml = hooksYaml(mission)
    // Follow the server text only while there are no unsaved edits; a stale save meets the revision conflict.
    const [baseYaml, setBaseYaml] = useState(savedYaml)
    if (savedYaml !== baseYaml) {
        setBaseYaml(savedYaml)
        if (yaml === baseYaml) setYaml(savedYaml)
    }
    const lastSeq = events.at(-1)?.seq ?? 0
    useEffect(() => {
        if (!started) return
        let disposed = false
        request<{ events: MissionEvent[] }>(project, mission.id, undefined, `/events${lastSeq ? `?after=${lastSeq}` : ''}`)
            .then(value => { if (!disposed) setEvents(prev => [...prev, ...value.events.filter(event => event.seq > (prev.at(-1)?.seq ?? 0))]) }).catch(() => {})
        return () => { disposed = true }
    }, [project, mission.id, started, mission.event_seq, mission.cursor, mission.runs?.length, mission.updated_at]) // eslint-disable-line react-hooks/exhaustive-deps
    async function act(work: () => Promise<Mission>) {
        if (pending || busy) return
        setPending(true); setError('')
        try { onChange(await work()) } catch (e) { setError(e instanceof MissionConflict ? 'This mission changed on the server. Review the latest hooks and budget, then save again.' : (e as Error).message) } finally { setPending(false) }
    }
    const control = (action: string, body: unknown = {}) => act(() => request<Mission>(project, mission.id, body, `/${action}`))
    function openRun(runId: string) {
        const state = useStore.getState()
        state.setRunsSelectedRunIdForScope(buildRunsScopeKey(state.runsListSession.scopeMode, state.activeProjectPath), runId)
        state.setViewMode('runs')
    }
    const disabled = pending || busy
    const substate = mission.execution?.substate ?? 'idle'
    return <div className="space-y-4">
        {error && <InlineError>{error}</InlineError>}
        <section aria-label="Controls" className="flex flex-wrap items-center gap-2 border-t border-border pt-4">
            {!started && <Button size="sm" disabled={disabled} onClick={() => void control('start')}>Start</Button>}
            {started && <>
                <span role="status" className={`rounded border px-2 py-1 text-xs ${substate === 'attention' ? 'border-destructive text-destructive' : 'border-border text-muted-foreground'}`}>{closed ? `Closed as ${mission.closed?.status}` : substateLabels[substate]}{mission.paused ? ' · Paused' : ''}</span>
                {!closed && !mission.paused && <Button size="sm" variant="secondary" disabled={disabled} onClick={() => void control('pause')}>Pause</Button>}
                {!closed && (mission.paused || substate === 'attention') && <Button size="sm" variant="secondary" disabled={disabled} onClick={() => void control('resume')}>Resume</Button>}
                {!closed && <Button size="sm" variant="secondary" disabled={disabled} onClick={() => void control('cancel')}>Cancel mission</Button>}
                {!closed && <Button size="sm" variant="secondary" disabled={disabled} onClick={() => void control('close', { status: 'done' })}>Close as done</Button>}
            </>}
        </section>
        {started && mission.execution?.reason && <p className="text-xs text-muted-foreground">{mission.execution.reason}</p>}
        {started && <>
            <section aria-label="State" className="space-y-2 border-t border-border pt-4">
                <h3 className="font-medium">State</h3>
                {mission.state ? <ProjectConversationMarkdown content={mission.state} /> : <p className="text-muted-foreground">No state recorded yet</p>}
            </section>
            <section aria-label="Runs" className="space-y-2 border-t border-border pt-4">
                <h3 className="font-medium">Runs</h3>
                {mission.runs?.length ? <ul className="space-y-1">{mission.runs.map(run => <li key={run.run_id}>
                    <Button variant="ghost" size="sm" className="h-auto w-full justify-between gap-2 whitespace-normal text-left" aria-label={`Open run ${run.label}`} onClick={() => openRun(run.run_id)}>
                        <span className="break-all">{run.label}{run.role === 'reaction' ? ' (reaction)' : ''}</span><span className="shrink-0 text-xs text-muted-foreground">{run.status}</span>
                    </Button></li>)}</ul>
                    : <Empty className="px-3 py-4 text-xs text-muted-foreground"><EmptyDescription>No runs yet</EmptyDescription></Empty>}
            </section>
            <section aria-label="Events" className="space-y-2 border-t border-border pt-4">
                <h3 className="font-medium">Events</h3>
                {events.length ? <ol className="space-y-1 text-xs">{events.slice(-20).map(event => <li key={event.seq} className="break-words"><span className="font-medium">{event.kind}</span> <span className="text-muted-foreground">· {event.source}</span>{describe(event) && <span className="block whitespace-pre-wrap">{describe(event)}</span>}</li>)}</ol>
                    : <Empty className="px-3 py-4 text-xs text-muted-foreground"><EmptyDescription>No events yet</EmptyDescription></Empty>}
                <form className="grid gap-2" onSubmit={e => {
                    e.preventDefault()
                    const text = message.trim()
                    if (text) void act(async () => { const saved = await request<Mission>(project, mission.id, { kind: 'human.message', payload: { message: text } }, '/events'); setMessage(''); return saved })
                }}>
                    <Label className="grid gap-2">Message<Textarea className="min-h-16" disabled={disabled} value={message} onChange={e => setMessage(e.target.value)} /></Label>
                    <Button type="submit" size="sm" className="justify-self-start" disabled={disabled || !message.trim()}>Send message</Button>
                </form>
            </section>
            <section aria-label="Hooks and budget" className="space-y-2 border-t border-border pt-4">
                <h3 className="font-medium">Hooks and budget</h3>
                <Label className="grid gap-2">YAML<Textarea className="min-h-32 font-mono text-xs" spellCheck={false} disabled={disabled} value={yaml} onChange={e => setYaml(e.target.value)} /></Label>
                <Button size="sm" disabled={disabled || yaml === savedYaml} onClick={() => void act(async () => { const saved = await request<Mission>(project, mission.id, { revision: mission.revision, yaml, actor: 'human' }); setYaml(hooksYaml(saved)); return saved })}>Save hooks and budget</Button>
            </section>
        </>}
    </div>
}
