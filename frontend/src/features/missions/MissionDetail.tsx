import { useStore } from '@/store'
import { fetchTriggerListValidated, type TriggerResponse } from '@/lib/api/triggersApi'
import { useInheritedModelSettings } from '@/components/model-chooser/useInheritedModelSettings'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { DropdownMenu } from 'radix-ui'
import { Check, MoreHorizontal, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { SectionHeading } from '@/components/app/section-heading'
import { StatusDot, StatusPill, statusTextClassName } from '@/components/app/status'
import { Well } from '@/components/app/well'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { MissionConflict, request, statusLabels, type Budget, type Mission, type Playbook } from './MissionsPanel'
import { MissionTranscript, RunLink } from './MissionTranscript'
import { useMissionConversation } from './hooks/useMissionConversation'
import { formatMissionDate, joinRuns, missionTone, readQuestion, runKinds, runStatusLabel, shortLine, totalTokens, type MissionRun } from './model/missionModel'
import { ModelChooser } from '@/components/model-chooser/ModelChooser'
import { ApiHttpError } from '@/lib/api/shared'
import { fetchConversationSnapshotValidated, updateConversationSettingsValidated } from '@/lib/api/conversationsApi'
import type { ModelSettings } from '@/lib/api/settingsApi'
import { useRunsList } from '@/features/runs/hooks/useRunsList'
import { flowTitle, formatCompactCount } from '@/features/runs/model/runOverviewModel'
import { projectLabel } from '@/features/projects/model/projectChoices'

type Props = {
    mission: Mission; project: string; busy: boolean; error: string; narrow: boolean; focusRequest: number
    edit: () => void; close: () => void; archive: (value: boolean) => Promise<void>; onChange: (mission: Mission) => void
}
const defaultBudget: Budget = { concurrent_runs: 4, total_runs: 25 }
const inheritedModel: ModelSettings = { provider: null, llm_profile: null, model: null, reasoning_effort: null }
const menuItem = 'relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50'
const closedActors: Record<string, string> = { assistant: 'the mission agent', human: 'you' }

function RailSection({ label, children, action }: { label: string; children: ReactNode; action?: ReactNode }) {
    return <section aria-label={label} className="flex flex-col gap-2">
        <SectionHeading as="h3" action={action}>{label}</SectionHeading>
        {children}
    </section>
}

/** The mission as a conversation with its agent, beside a rail of its objective, runs, budget and model. */
export function MissionDetail({ mission, project, busy, error, narrow, focusRequest, edit, close, archive, onChange }: Props) {
    const registry = useStore(s => s.projectRegistry)
    const [triggers, setTriggers] = useState<TriggerResponse[]>([])
    const [triggerError, setTriggerError] = useState('')
    useEffect(() => {
        let disposed = false
        const load = () => { void fetchTriggerListValidated().then(items => { if (!disposed) { setTriggers(items.filter(t => t.action.mode === 'mission' && t.action.mission_id === mission.id && t.action.project_path === project)); setTriggerError('') } }).catch(error => { if (!disposed) setTriggerError(error.message) }) }
        load()
        window.addEventListener('spark:trigger-live-event', load)
        return () => { disposed = true; window.removeEventListener('spark:trigger-live-event', load) }
    }, [mission.id, project])
    const heading = useRef<HTMLHeadingElement>(null)
    useEffect(() => { heading.current?.focus({ preventScroll: true }) }, [mission.id, focusRequest])
    const [pending, setPending] = useState(false)
    const [actionError, setActionError] = useState('')
    const [reply, setReply] = useState('')
    const [budget, setBudget] = useState<Budget | null>(null)
    const [model, setModel] = useState<{ revision: number; draft: ModelSettings | null } | null>(null)
    const [objectiveOpen, setObjectiveOpen] = useState(false)
    const [objectiveClipped, setObjectiveClipped] = useState(false)
    const objectiveText = useRef<HTMLParagraphElement>(null)
    useEffect(() => {
        const element = objectiveText.current
        if (!element) return
        const measure = () => setObjectiveClipped(element.scrollHeight > element.clientHeight)
        measure()
        const observer = new ResizeObserver(measure)
        observer.observe(element)
        return () => observer.disconnect()
    }, [mission.fields.description])
    // A draft names its playbook; Start freezes the text into mission.playbook.
    const draftPlaybook = mission.playbook ? null : mission.fields.playbook
    const [loadedPlaybook, setLoadedPlaybook] = useState<Playbook | null>(null)
    useEffect(() => {
        setLoadedPlaybook(null)
        if (!draftPlaybook) return
        let disposed = false
        void fetch(`/workspace/api/playbooks/${encodeURIComponent(draftPlaybook)}?project_path=${encodeURIComponent(project)}`).then(response => response.ok ? response.json() : null).then(value => { if (!disposed) setLoadedPlaybook(value) }).catch(() => {})
        return () => { disposed = true }
    }, [draftPlaybook, project])
    const playbook = mission.playbook ?? (loadedPlaybook?.name === draftPlaybook ? loadedPlaybook : null)
    const conversation = useMissionConversation(mission, project)
    const { conversationId } = conversation
    const inherited = useInheritedModelSettings(project)
    const { scopedRuns } = useRunsList({ selectedRunId: null, manageSync: false })
    const runs: MissionRun[] = joinRuns(mission.runs ?? [], scopedRuns)
    const runsById = new Map(runs.map(run => [run.runId, run]))
    const tokens = totalTokens(runs)
    async function openModel() {
        setActionError('')
        try {
            const snapshot = await fetchConversationSnapshotValidated(conversationId, project)
            setModel({ revision: snapshot.revision, draft: snapshot.model_settings_view?.effective ?? inheritedModel })
        } catch (e) {
            if (e instanceof ApiHttpError && e.status === 404) setModel({ revision: 0, draft: inheritedModel })
            else setActionError((e as Error).message)
        }
    }
    async function saveModel() {
        if (!model || disabled) return
        setPending(true); setActionError('')
        try {
            await updateConversationSettingsValidated(conversationId, { project_path: project, expected_revision: String(model.revision), model_settings: model.draft })
            setModel(null); conversation.reload()
        } catch (e) { setActionError((e as Error).message) } finally { setPending(false) }
    }
    const status = mission.status ?? 'draft'
    const closed = status === 'closed'
    // A closed mission's short line already leads with its state word.
    const [stateWord, ...rest] = closed ? shortLine(mission).split(' · ') : [statusLabels[status], shortLine(mission)]
    let lineRest = rest.join(' · ')
    // A closed mission's header spans from its start to its close.
    const started = closed ? formatMissionDate(mission.started_at) : ''
    if (started && lineRest && started !== lineRest) lineRest = `${started} – ${lineRest}`
    const disabled = busy || pending
    async function act(work: () => Promise<Mission>, control = false) {
        if (disabled) return false
        setPending(true); setActionError('')
        try { onChange(await work()); return true } catch (e) {
            setActionError(e instanceof MissionConflict && !control ? 'This mission changed on the server. Review the latest values, then try again.' : (e as Error).message)
            return false
        } finally { setPending(false) }
    }
    const control = (action: string, body: unknown = {}) => act(() => request<Mission>(project, mission.id, body, `/${action}`), true)
    const send = (message: string, clear = false) => act(async () => {
        const saved = await request<Mission>(project, mission.id, { kind: 'human.message', payload: { message } }, '/events')
        if (clear) setReply('')
        return saved
    })
    const question = status === 'needs_you' ? readQuestion(mission.question) : null
    const used = mission.runs?.length ?? 0
    const limits = mission.fields.budget ?? defaultBudget
    const stored = conversation.snapshot?.model_settings_view?.stored
    const modelLabel = [stored?.model || stored?.llm_profile || stored?.provider, stored?.reasoning_effort].filter(Boolean).join(' · ') || 'Project default'
    const objective = mission.fields.description || 'No objective'
    const closedActor = mission.closed?.actor ? closedActors[mission.closed.actor] ?? mission.closed.actor : ''
    return <section aria-label="Mission details" className="flex min-h-0 min-w-0 flex-1 flex-col" onKeyDown={e => {
        // Menu keys arrive through the portal; only the pane's own Escape dismisses it.
        if (e.key === 'Escape' && !e.defaultPrevented && !e.nativeEvent.isComposing && !busy && e.currentTarget.contains(e.target as Node)) { e.stopPropagation(); close() }
    }}>
        <header className="flex shrink-0 flex-wrap items-start justify-between gap-x-6 gap-y-3 border-b border-border px-6 pt-5 pb-4">
            <div className="flex min-w-0 flex-1 basis-80 flex-col gap-1.5">
                <p className="truncate text-xs text-muted-foreground">{projectLabel(registry, project)} · Mission</p>
                <h2 ref={heading} tabIndex={-1} title={mission.fields.title} className="line-clamp-2 max-w-[62ch] text-xl leading-snug font-semibold break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{mission.fields.title}</h2>
                <div role="status" className="flex min-w-0 items-center gap-2.5">
                    <StatusPill tone={missionTone(mission)}>{stateWord}</StatusPill>
                    {' '}
                    <span className="min-w-0 truncate text-sm text-muted-foreground">{lineRest}{mission.fields.archived ? ' · Archived' : ''}{busy || pending ? ' · Saving…' : ''}</span>
                </div>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
                {status === 'draft' && <><Button type="button" size="sm" disabled={disabled} onClick={() => void control('start')}>Start</Button>
                    <Button type="button" size="sm" variant="outline" disabled={disabled} onClick={edit}>Edit</Button></>}
                {(status === 'running' || status === 'needs_you') && <><Button type="button" size="sm" variant="outline" aria-label="Close mission" disabled={disabled} onClick={() => void control('close', { status: 'done' })}>Close</Button>
                    <Button type="button" size="sm" variant="ghost" aria-label="Cancel mission" disabled={disabled} onClick={() => void control('cancel')}>Cancel</Button></>}
                {closed && <Button type="button" size="sm" variant="outline" disabled={disabled} onClick={() => void archive(!mission.fields.archived)}>{mission.fields.archived ? 'Restore' : 'Archive'}</Button>}
                <DropdownMenu.Root>
                    <DropdownMenu.Trigger asChild><Button type="button" variant="ghost" size="icon-sm" aria-label="Mission actions" disabled={disabled}><MoreHorizontal aria-hidden="true" className="size-4" /></Button></DropdownMenu.Trigger>
                    <DropdownMenu.Portal>
                        <DropdownMenu.Content align="end" sideOffset={4} className="z-50 min-w-40 rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md">
                            <DropdownMenu.Item className={menuItem} onSelect={edit}>Edit</DropdownMenu.Item>
                            <DropdownMenu.Item className={menuItem} disabled={closed} onSelect={() => void openModel()}>Model</DropdownMenu.Item>
                            <DropdownMenu.Item className={menuItem} onSelect={() => setBudget(limits)}>Budget</DropdownMenu.Item>
                            <DropdownMenu.Item className={menuItem} onSelect={() => void archive(!mission.fields.archived)}>{mission.fields.archived ? 'Restore' : 'Archive'}</DropdownMenu.Item>
                        </DropdownMenu.Content>
                    </DropdownMenu.Portal>
                </DropdownMenu.Root>
                <Button type="button" variant="ghost" size="icon-sm" aria-label="Close details" disabled={busy} onClick={close}><X aria-hidden="true" className="size-4" /></Button>
            </div>
        </header>
        {(error || actionError) && <div className="shrink-0 px-6 pt-4"><InlineError>{error || actionError}</InlineError></div>}
        <div data-testid="mission-body" className={narrow ? 'min-h-0 flex-1 overflow-y-auto' : 'flex min-h-0 flex-1'}>
            <div data-testid="mission-conversation" className={`flex min-w-0 flex-col ${narrow ? '' : 'min-h-0 flex-1'}`}>
                <div className={`px-6 py-5 ${narrow ? '' : 'min-h-0 flex-1 overflow-y-auto'}`}>
                    <div className="flex max-w-3xl flex-col gap-5 text-sm">
                        {closed && mission.closed && <Well role="region" aria-label="Outcome" className="flex flex-col gap-2.5">
                            <div className="flex flex-wrap items-center gap-2">
                                {mission.closed.status === 'done'
                                    ? <Check aria-hidden="true" className="size-4 text-success" />
                                    : <X aria-hidden="true" className={`size-4 ${statusTextClassName(mission.closed.status === 'failed' ? 'destructive' : 'neutral')}`} />}
                                <h3 className="font-semibold">Closed as {mission.closed.status}</h3>
                                {' '}
                                <span className="text-muted-foreground">· <time dateTime={mission.closed.at}>{formatMissionDate(mission.closed.at)}</time>{closedActor && `, by ${closedActor}`}</span>
                            </div>
                            {mission.closed.reason && <p className="max-w-[72ch] leading-relaxed whitespace-pre-wrap break-words">{mission.closed.reason}</p>}
                        </Well>}
                        {status === 'draft'
                            ? <Empty><EmptyDescription>No conversation yet. Start the mission and its agent picks up the objective.</EmptyDescription></Empty>
                            : <MissionTranscript conversation={conversation} project={project} runs={runsById} />}
                        {question && <Well role="region" aria-label="Waiting on you" className="flex flex-col gap-2">
                            <p className={`text-xs font-medium ${statusTextClassName('warning')}`}>Waiting on you{question.runId && <> · from <RunLink runId={question.runId} className="text-foreground">{runsById.get(question.runId)?.title || flowTitle(question.flowName)}</RunLink></>}</p>
                            <p className="font-medium whitespace-pre-wrap break-words">{question.prompt}</p>
                            {question.options.length > 0 && <div className="mt-1 flex flex-wrap gap-2">{question.options.map(option => <Button key={option} type="button" size="sm" variant="outline" disabled={disabled} onClick={() => void send(option)}>{option}</Button>)}</div>}
                        </Well>}
                    </div>
                </div>
                {!closed && <form className="flex shrink-0 items-end gap-2 px-6 pb-5" onSubmit={e => {
                    e.preventDefault()
                    const text = reply.trim()
                    if (text) void send(text, true)
                }}>
                    <Textarea aria-label="Reply" rows={1} className="min-h-9 max-w-3xl flex-1 resize-none" placeholder={status === 'draft' ? 'Start the mission to talk to its agent' : 'Message the mission’s agent…'} disabled={disabled || status === 'draft'} value={reply} onChange={e => setReply(e.target.value)} onKeyDown={e => {
                        if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); e.currentTarget.form?.requestSubmit() }
                    }} />
                    {/* Start is a draft's one primary button; Send takes over once the mission runs. */}
                    <Button type="submit" size="sm" variant={status === 'draft' ? 'outline' : 'default'} disabled={disabled || status === 'draft' || !reply.trim()}>Send</Button>
                </form>}
            </div>
            <aside aria-label="Mission overview" className={narrow ? 'px-6 pb-6' : 'w-80 shrink-0 overflow-y-auto py-5 pr-6'}>
                <Well className="flex flex-col gap-5 text-sm">
                    <RailSection label="Objective">
                        <p ref={objectiveText} tabIndex={0} className={`leading-relaxed whitespace-pre-wrap break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${objectiveOpen ? '' : 'line-clamp-4'}`}>{objective}</p>
                        {(objectiveOpen || objectiveClipped) && <Button type="button" variant="ghost" size="xs" className="-ml-2 self-start" aria-expanded={objectiveOpen} onClick={() => setObjectiveOpen(!objectiveOpen)}>{objectiveOpen ? 'Show less' : 'Show full objective'}</Button>}
                    </RailSection>
                    {(mission.playbook || mission.fields.playbook) && <RailSection label="Playbook">
                        {playbook
                            ? <details><summary className="cursor-pointer rounded-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{playbook.name}</summary>
                                <p className="mt-1 max-h-64 overflow-y-auto text-xs leading-relaxed whitespace-pre-wrap break-words">{playbook.text}</p></details>
                            : <p>{mission.fields.playbook}</p>}
                    </RailSection>}
                    <RailSection label="Runs">
                        {runs.length
                            ? <ul className="-mx-2 flex flex-col">{runs.map(run => <li key={run.runId} className="flex items-start gap-2.5 rounded-lg px-2 py-1.5">
                                <StatusDot tone={runKinds[run.kind].tone} className="mt-[7px]" />
                                <div className="min-w-0 flex-1 leading-5">
                                    <RunLink runId={run.runId} className="block truncate font-medium text-foreground">{run.title}</RunLink>
                                    <div className="text-xs text-muted-foreground">{run.flowTitle}{run.duration && ` · ${run.duration}`}{(run.kind === 'failed' || run.kind === 'waiting') && <> · <span className={statusTextClassName(runKinds[run.kind].tone)}>{runStatusLabel(run)}</span></>}</div>
                                </div>
                            </li>)}</ul>
                            : <p className="text-xs text-muted-foreground">None yet</p>}
                    </RailSection>
                    <RailSection label="Budget" action={!budget && <Button type="button" variant="ghost" size="xs" aria-label="Edit budget" disabled={disabled} onClick={() => setBudget(limits)}>Edit</Button>}>
                        {budget
                            ? <form aria-label="Budget" className="grid gap-3" onSubmit={e => {
                                e.preventDefault()
                                void act(() => request<Mission>(project, mission.id, { revision: mission.revision, fields: { budget }, actor: 'human' })).then(saved => { if (saved) setBudget(null) })
                            }}>
                                <Label className="grid gap-1 text-xs">Concurrent runs<Input autoFocus type="number" min={1} required disabled={disabled} className="bg-background" value={budget.concurrent_runs} onChange={e => setBudget({ ...budget, concurrent_runs: Number(e.target.value) })} /></Label>
                                <Label className="grid gap-1 text-xs">Total runs<Input type="number" min={1} required disabled={disabled} className="bg-background" value={budget.total_runs} onChange={e => setBudget({ ...budget, total_runs: Number(e.target.value) })} /></Label>
                                <div className="flex gap-2"><Button type="submit" size="sm" variant="outline" disabled={disabled}>Save budget</Button><Button type="button" size="sm" variant="ghost" disabled={disabled} onClick={() => setBudget(null)}>Cancel</Button></div>
                            </form>
                            : <>
                                <p className="text-xs">{used} of {limits.total_runs} runs · up to {limits.concurrent_runs} at once</p>
                                <div role="meter" aria-label="Runs used" aria-valuemin={0} aria-valuemax={limits.total_runs} aria-valuenow={used} className="h-0.5 rounded-full bg-background"><div className="h-0.5 rounded-full bg-primary" style={{ width: `${Math.min(100, used / Math.max(1, limits.total_runs) * 100)}%` }} /></div>
                            </>}
                    </RailSection>
                    <RailSection label="Model" action={!model && <Button type="button" variant="ghost" size="xs" aria-label="Change model" disabled={disabled || closed} onClick={() => void openModel()}>Change</Button>}>
                        {model
                            ? <form aria-label="Model" className="grid gap-3" onSubmit={e => { e.preventDefault(); void saveModel() }}>
                                <ModelChooser inherited={inherited} projectPath={project} inheritLabel="Project default" disabled={disabled}
                                    value={model.draft ?? inheritedModel} onChange={draft => setModel(current => current && ({ ...current, draft }))} />
                                <p className="text-xs text-muted-foreground">Applies from the mission's next turn.</p>
                                <div className="flex gap-2"><Button type="submit" size="sm" variant="outline" disabled={disabled}>Save model</Button><Button type="button" size="sm" variant="ghost" disabled={disabled} onClick={() => setModel(null)}>Cancel</Button></div>
                            </form>
                            : <p className="text-xs">{modelLabel}</p>}
                    </RailSection>
                    {tokens !== null && <RailSection label="Spent"><p className="text-xs">{formatCompactCount(tokens)} tokens across runs</p></RailSection>}
                    {triggerError && <InlineError>{triggerError}</InlineError>}
                    {triggers.length > 0 && <RailSection label="Triggers"><ul aria-label="Targeting triggers">{triggers.map(trigger => <li key={trigger.id}><button type="button" className="text-left text-xs text-primary hover:underline" onClick={() => { useStore.getState().updateTriggersSession({ selectedTriggerId: trigger.id }); useStore.getState().setViewMode('triggers') }}>{trigger.name} · {trigger.source_type} · {trigger.enabled ? 'Enabled' : 'Disabled'}</button></li>)}</ul></RailSection>}
                </Well>
            </aside>
        </div>
    </section>
}
