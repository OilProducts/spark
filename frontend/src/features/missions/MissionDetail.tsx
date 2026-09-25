import { useEffect, useRef, useState } from 'react'
import { DropdownMenu } from 'radix-ui'
import { MoreHorizontal, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { InlineError } from '@/components/app/inline-error'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { MissionConflict, request, statusLabels, statusLine, type Budget, type Mission } from './MissionsPanel'
import { MissionTranscript } from './MissionTranscript'

type Props = {
    mission: Mission; project: string; busy: boolean; error: string; narrow: boolean; focusRequest: number
    edit: () => void; close: () => void; archive: (value: boolean) => Promise<void>; onChange: (mission: Mission) => void
}
const defaultBudget: Budget = { concurrent_runs: 4, total_runs: 25 }
const menuItem = 'relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50'

/** The mission's transcript: the pinned objective, the conversation, and the reply box. */
export function MissionDetail({ mission, project, busy, error, narrow, focusRequest, edit, close, archive, onChange }: Props) {
    const heading = useRef<HTMLHeadingElement>(null)
    useEffect(() => { heading.current?.focus({ preventScroll: true }) }, [mission.id, focusRequest])
    const [pending, setPending] = useState(false)
    const [actionError, setActionError] = useState('')
    const [reply, setReply] = useState('')
    const [budget, setBudget] = useState<Budget | null>(null)
    const status = mission.status ?? 'draft'
    const closed = status === 'closed'
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
    return <section aria-label="Mission details" className={`flex min-h-0 min-w-0 flex-col rounded-md border border-border bg-card ${narrow ? 'w-full' : 'flex-1'}`} onKeyDown={e => {
        // Menu keys arrive through the portal; only the pane's own Escape dismisses it.
        if (e.key === 'Escape' && !e.defaultPrevented && !e.nativeEvent.isComposing && !busy && e.currentTarget.contains(e.target as Node)) { e.stopPropagation(); close() }
    }}>
        <header className="flex shrink-0 items-start gap-2 border-b border-border p-4">
            <div className="min-w-0 flex-1">
                <h2 ref={heading} tabIndex={-1} className="text-lg font-semibold whitespace-pre-wrap break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{mission.fields.title}</h2>
                <p role="status" className="mt-1 text-xs text-muted-foreground"><span className="font-medium text-foreground">{statusLabels[status]}</span> · {statusLine(mission)}{busy || pending ? ' · Saving…' : ''}</p>
                {mission.fields.archived && <span className="mt-2 inline-block rounded border border-border px-2 py-0.5 text-xs text-muted-foreground">Archived</span>}
            </div>
            <DropdownMenu.Root>
                <DropdownMenu.Trigger asChild><Button type="button" variant="ghost" size="icon-sm" aria-label="Mission actions" disabled={disabled}><MoreHorizontal aria-hidden="true" className="size-4" /></Button></DropdownMenu.Trigger>
                <DropdownMenu.Portal>
                    <DropdownMenu.Content align="end" sideOffset={4} className="z-50 min-w-40 rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md">
                        <DropdownMenu.Item className={menuItem} onSelect={edit}>Edit</DropdownMenu.Item>
                        <DropdownMenu.Item className={menuItem} onSelect={() => setBudget(mission.fields.budget ?? defaultBudget)}>Budget</DropdownMenu.Item>
                        <DropdownMenu.Item className={menuItem} disabled={closed} onSelect={() => void control('cancel')}>Cancel mission</DropdownMenu.Item>
                        <DropdownMenu.Item className={menuItem} disabled={closed} onSelect={() => void control('close', { status: 'done' })}>Close mission</DropdownMenu.Item>
                        <DropdownMenu.Item className={menuItem} onSelect={() => void archive(!mission.fields.archived)}>{mission.fields.archived ? 'Restore' : 'Archive'}</DropdownMenu.Item>
                    </DropdownMenu.Content>
                </DropdownMenu.Portal>
            </DropdownMenu.Root>
            <Button type="button" variant="ghost" size="icon-sm" aria-label="Close details" disabled={busy} onClick={close}><X aria-hidden="true" className="size-4" /></Button>
        </header>
        {(error || actionError) && <div className="shrink-0 px-4 pt-3"><InlineError>{error || actionError}</InlineError></div>}
        {budget && <form aria-label="Budget" className="grid shrink-0 gap-3 border-b border-border p-4 sm:grid-cols-[1fr_1fr_auto] sm:items-end" onSubmit={e => {
            e.preventDefault()
            void act(() => request<Mission>(project, mission.id, { revision: mission.revision, fields: { budget }, actor: 'human' })).then(saved => { if (saved) setBudget(null) })
        }}>
            <Label className="grid gap-2">Concurrent runs<Input type="number" min={1} required disabled={disabled} value={budget.concurrent_runs} onChange={e => setBudget({ ...budget, concurrent_runs: Number(e.target.value) })} /></Label>
            <Label className="grid gap-2">Total runs<Input type="number" min={1} required disabled={disabled} value={budget.total_runs} onChange={e => setBudget({ ...budget, total_runs: Number(e.target.value) })} /></Label>
            <div className="flex gap-2"><Button type="submit" size="sm" disabled={disabled}>Save budget</Button><Button type="button" size="sm" variant="ghost" disabled={disabled} onClick={() => setBudget(null)}>Cancel</Button></div>
        </form>}
        <div className="min-h-0 flex-1 overflow-y-auto text-sm">
            <section aria-label="Objective" className="sticky top-0 z-10 border-b border-border bg-card p-4">
                <h3 className="text-xs font-medium text-muted-foreground">Objective</h3>
                <p tabIndex={0} className="mt-1 max-h-32 overflow-y-auto whitespace-pre-wrap break-words leading-relaxed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{mission.fields.description || 'No objective'}</p>
            </section>
            {status !== 'draft' && mission.conversation_id && <div className="p-4"><MissionTranscript mission={mission} project={project} /></div>}
        </div>
        <footer className="shrink-0 border-t border-border p-4">
            {status === 'draft'
                ? <Button type="button" disabled={disabled} onClick={() => void control('start')}>Start</Button>
                : closed
                    ? <p className="text-xs text-muted-foreground">This mission is closed.</p>
                    : <form className="flex items-end gap-2" onSubmit={e => {
                        e.preventDefault()
                        const text = reply.trim()
                        if (text) void act(async () => { const saved = await request<Mission>(project, mission.id, { kind: 'human.message', payload: { message: text } }, '/events'); setReply(''); return saved })
                    }}>
                        <Label className="grid min-w-0 flex-1 gap-2">Reply<Textarea className="min-h-16" disabled={disabled} value={reply} onChange={e => setReply(e.target.value)} /></Label>
                        <Button type="submit" disabled={disabled || !reply.trim()}>Send</Button>
                    </form>}
        </footer>
    </section>
}
