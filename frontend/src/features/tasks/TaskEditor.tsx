import { useEffect, useRef } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { NativeSelect } from '@/components/ui/native-select'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import { stages, labels, type Task, type Fields, type Board, type Stage } from './TasksPanel'

type Props = {
    editing: Task | null; draft: Fields; note: string; board: Board; latest?: Task
    busy: boolean; conflict: boolean; error: string; unsaved: boolean; narrow: boolean; focusRequest: number
    setDraft: (fields: Fields) => void; setNote: (note: string) => void
    save: () => Promise<void>; close: () => void; discard: () => void; reconcile: () => void
    run: (id: string) => void; conversation: (id: string) => void
}
export function TaskEditor({ editing, draft, note, board, latest, busy, conflict, error, unsaved, narrow, focusRequest, setDraft, setNote, save, close, discard, reconcile, run, conversation }: Props) {
    const heading = useRef<HTMLHeadingElement>(null)
    const title = useRef<HTMLInputElement>(null)
    const saved = Boolean(editing)
    useEffect(() => { if (saved) heading.current?.focus(); else title.current?.focus() }, [saved, focusRequest])
    const changed = latest && latest.revision > (editing?.revision ?? 0)
    const completion = draft.stage === 'done' && editing?.fields.stage !== 'done'
    const hasResources = Boolean(editing && (editing.fields.runs.length || editing.fields.conversations.length || editing.fields.artifacts.length || board.attention.some(a => a.task_id === editing.id)))
    const textField = (key: 'description' | 'acceptance_criteria' | 'next_action' | 'blocked' | 'needs_input', label: string) => <Label className="grid gap-2" key={key}>{label}<Textarea value={draft[key]} onChange={e => setDraft({ ...draft, [key]: e.target.value })} /></Label>
    return <section aria-label="Task details" className={`flex min-h-0 min-w-0 flex-col rounded border bg-background ${narrow ? 'w-full' : 'w-[28rem] shrink-0'}`} onKeyDown={e => {
        if (e.key === 'Escape' && !e.defaultPrevented && !e.nativeEvent.isComposing && !busy) { e.stopPropagation(); close() }
    }}>
        <header className="shrink-0 border-b p-4">
            <h2 ref={heading} tabIndex={-1} className="font-semibold break-words">{editing?.fields.title ?? 'New task'}</h2>
            {editing && <p className="text-xs text-muted-foreground">{editing.id} · revision {editing.revision}</p>}
            {unsaved && <p className="text-sm">Unsaved changes</p>}
            <p className="text-xs text-muted-foreground">Drafts remain available during this workspace session.</p>
        </header>
        <form className="flex min-h-0 flex-1 flex-col" onSubmit={e => { e.preventDefault(); void save() }}>
        <div className="min-h-0 flex-1 overflow-y-auto p-4 space-y-4">
            {error && <p role="alert">{error}</p>}
            {conflict && !changed && <p role="status">Refresh to load the latest revision and reconcile before saving. Your draft is preserved.</p>}
            {changed && <div role="status">A newer revision is available. Your draft is preserved. Review the server changes below before reconciling; your edited fields take precedence. <Button variant="secondary" type="button" disabled={busy} onClick={reconcile}>Reconcile with latest revision</Button><pre className="whitespace-pre-wrap break-words">{JSON.stringify(latest.fields, null, 2)}</pre></div>}
            <fieldset disabled={busy} className="grid min-w-0 gap-3">
                <Label className="grid gap-2">Title<Input ref={title} required value={draft.title} onChange={e => setDraft({ ...draft, title: e.target.value })} /></Label>{textField('description', 'Outcome and decisions')}{textField('next_action', 'Next action')}
                <Label>Stage <NativeSelect className="border bg-background p-2" value={draft.stage} onChange={e => setDraft({ ...draft, stage: e.target.value as Stage })}>{stages.map((s, i) => <option key={s} value={s}>{labels[i]}</option>)}</NativeSelect></Label>
                <Label>Priority <NativeSelect className="border bg-background p-2" value={draft.priority} onChange={e => setDraft({ ...draft, priority: Number(e.target.value) })}>{['Urgent', 'High', 'Normal', 'Low'].map((p, i) => <option key={p} value={i}>{p}</option>)}</NativeSelect></Label>
                <details open={Boolean(draft.blocked || draft.needs_input)}><summary>Blockers and questions</summary><div className="grid gap-3 pt-3">{textField('blocked', 'Blocked — explanation (empty to clear)')}{textField('needs_input', 'Needs input — question (empty to clear)')}</div></details>
                <details><summary>Acceptance criteria</summary>{textField('acceptance_criteria', 'Acceptance criteria')}</details>
                <details><summary>Archival</summary><Label className="py-3"><Checkbox disabled={busy} checked={draft.archived} onCheckedChange={value => setDraft({ ...draft, archived: value === true })} /> Archived</Label></details>
                <details><summary>Relationships</summary><div className="grid gap-3 pt-3">
                <Label className="grid">Conversation IDs (one per line)<Textarea className="border bg-background p-2" value={draft.conversations.join('\n')} onChange={e => setDraft({ ...draft, conversations: e.target.value.split('\n') })} /></Label>
                <Label className="grid">Repository artifact paths (one per line)<Textarea className="border bg-background p-2" value={draft.artifacts.join('\n')} onChange={e => setDraft({ ...draft, artifacts: e.target.value.split('\n') })} /></Label>
                <Label className="grid">Run IDs (one per line)<Textarea className="border bg-background p-2" value={draft.runs.map(r => r.run_id).join('\n')} onChange={e => setDraft({ ...draft, runs: e.target.value.split('\n').map(id => draft.runs.find(r => r.run_id === id) ?? { run_id: id, stage: null }) })} /></Label>
                {draft.runs.map((r, i) => r.run_id && <Label key={i}>Stage supported by {r.run_id} <NativeSelect aria-label={`Stage supported by ${r.run_id}`} value={r.stage ?? ''} onChange={e => setDraft({ ...draft, runs: draft.runs.map((link, index) => index === i ? { ...link, stage: e.target.value ? e.target.value as Stage : null } : link) })}><option value="">Unspecified</option>{stages.map((s, index) => <option key={s} value={s}>{labels[index]}</option>)}</NativeSelect></Label>)}
                </div></details>
                <details open={completion || Boolean(note)}><summary>Note / completion evidence</summary><Label className="grid gap-2 pt-3">Note / completion evidence<Textarea className="border bg-background p-2" value={note} required={completion} onChange={e => setNote(e.target.value)} /></Label></details>
            </fieldset>
            {hasResources && <div className="space-y-2"><h3>Related resources</h3>
            {editing?.fields.runs.map(r => <Button variant="link" disabled={busy} className="block h-auto max-w-full whitespace-normal text-left break-words" key={r.run_id} type="button" onClick={() => run(r.run_id)}>Open run {r.run_id} — {board.runs.find(run => run.run_id === r.run_id)?.status ?? 'Unavailable'}</Button>)}
            {board.attention.filter(a => a.task_id === editing?.id).map(a => <Button variant="link" disabled={busy} className="block h-auto max-w-full whitespace-normal text-left break-words" key={a.run_id} type="button" onClick={() => run(a.run_id)}>Answer questions in run {a.run_id}</Button>)}
            {editing?.fields.conversations.map(id => <Button variant="link" disabled={busy} className="block h-auto max-w-full whitespace-normal text-left break-words" key={id} type="button" onClick={() => conversation(id)}>Open conversation {id}</Button>)}
            {editing?.fields.artifacts.map(path => <p key={path}>Repository artifact: <code>{path}</code></p>)}
            </div>}
            {editing && <div><h3>Activity</h3><ol>{(latest ?? editing)?.activity.map(a => <li key={a.revision} className="border-b py-2">Revision {a.revision} · {a.actor} · {a.at}{a.conversation_id && ` · conversation ${a.conversation_id}`}<p>{a.note}</p>{a.associated_run_id && <p>Linked run {a.associated_run_id}</p>}{a.after && <details><summary>Changes</summary><pre className="whitespace-pre-wrap">{JSON.stringify(Object.fromEntries(Object.entries(a.after).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(a.before?.[key as keyof Fields]))), null, 2)}</pre></details>}</li>)}</ol></div>}
        </div>
        <footer className="flex shrink-0 flex-wrap gap-2 border-t p-4">
            <Button type="submit" disabled={busy || Boolean(changed) || conflict}>{busy ? 'Saving…' : editing ? 'Save task' : 'Create task'}</Button>
            <Button type="button" variant="secondary" disabled={busy} onClick={close}>Close</Button>
            <Button type="button" variant="ghost" disabled={busy} onClick={discard}>Discard changes</Button>
        </footer>
        </form>
    </section>
}
