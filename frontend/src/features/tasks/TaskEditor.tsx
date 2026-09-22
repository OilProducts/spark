import { useEffect, useRef } from 'react'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Label } from '@/components/ui/label'
import { stages, labels, type Task, type Fields, type Stage } from './TasksPanel'

type Props = {
    editing: Task | null; draft: Fields; latest?: Task
    busy: boolean; conflict: boolean; error: string; unsaved: boolean; narrow: boolean; focusRequest: number
    setDraft: (fields: Fields) => void
    save: () => Promise<void>; archive: () => Promise<void>; close: () => void; discard: () => void; reconcile: () => void
}
const fieldLabels: Record<keyof Fields, string> = { title: 'Title', description: 'Description', stage: 'Stage', archived: 'Archived' }
function display(key: keyof Fields, value: Fields[keyof Fields]) {
    if (key === 'stage') return labels[stages.indexOf(value as Stage)]
    if (typeof value === 'boolean') return value ? 'Yes' : 'No'
    return value || 'Empty'
}
export function TaskEditor({ editing, draft, latest, busy, conflict, error, unsaved, narrow, focusRequest, setDraft, save, archive, close, discard, reconcile }: Props) {
    const heading = useRef<HTMLHeadingElement>(null)
    const title = useRef<HTMLInputElement>(null)
    useEffect(() => { title.current?.focus({ preventScroll: true }) }, [focusRequest])
    const changed = latest && latest.revision > (editing?.revision ?? 0)
    return <section aria-label="Task details" className={`flex min-h-0 min-w-0 flex-col rounded-md border border-border bg-card ${narrow ? 'w-full' : 'w-[28rem] shrink-0'}`} onKeyDown={e => {
        if (e.key === 'Escape' && !e.defaultPrevented && !e.nativeEvent.isComposing && !busy) { e.stopPropagation(); close() }
    }}>
        <header className="shrink-0 border-b border-border p-4">
            <h2 ref={heading} tabIndex={-1} className="text-base font-semibold break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{editing?.fields.title ?? 'New task'}</h2>
            {unsaved && <p className="mt-1 text-xs text-muted-foreground">Unsaved changes</p>}
        </header>
        <form className="flex min-h-0 flex-1 flex-col" onSubmit={e => { e.preventDefault(); void save() }}>
        <div className="min-h-0 flex-1 overflow-y-auto p-4 space-y-4 text-sm">
            {error && <InlineError>{error}</InlineError>}
            {conflict && !changed && <p role="status">Refresh to load the latest revision and reconcile before saving. Your draft is preserved.</p>}
            {changed && <div role="status" className="space-y-3 rounded-md border border-border bg-muted/20 p-3">
                <p>A newer revision is available. Your draft is preserved. Review the latest values; your edited fields take precedence when reconciling.</p>
                <dl className="space-y-2">{(Object.keys(fieldLabels) as (keyof Fields)[]).filter(key => latest.fields[key] !== editing?.fields[key]).map(key => <div key={key}><dt className="font-medium">{fieldLabels[key]}</dt><dd className="whitespace-pre-wrap break-words text-muted-foreground">{display(key, latest.fields[key])}</dd></div>)}</dl>
                <Button variant="secondary" type="button" disabled={busy} onClick={reconcile}>Reconcile with latest revision</Button>
            </div>}
            <fieldset disabled={busy} className="grid min-w-0 gap-4">
                <Label className="grid gap-2">Title<Input ref={title} required value={draft.title} onChange={e => setDraft({ ...draft, title: e.target.value })} /></Label>
                <Label className="grid gap-2">Description<Textarea className="min-h-32" value={draft.description} onChange={e => setDraft({ ...draft, description: e.target.value })} /></Label>
                <Label className="grid gap-2">Stage<NativeSelect value={draft.stage} onChange={e => setDraft({ ...draft, stage: e.target.value as Stage })}>{stages.map((s, i) => <NativeSelectOption key={s} value={s}>{labels[i]}</NativeSelectOption>)}</NativeSelect></Label>
            </fieldset>
            {editing && <div className="border-t border-border pt-4"><Button type="button" variant="secondary" disabled={busy || Boolean(changed) || conflict} onClick={() => void archive()}>{editing.fields.archived ? 'Restore task' : 'Archive task'}</Button></div>}
            {editing && <details className="border-t border-border pt-4"><summary className="cursor-pointer rounded-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">Activity</summary><ol className="mt-2 space-y-3">{(latest ?? editing).activity.map(a => <li key={a.revision} className="border-b border-border pb-3">
                <p className="text-xs text-muted-foreground">{a.actor === 'assistant' ? 'Assistant' : 'Human'} · <time>{a.at}</time></p>
                {a.note && <p className="mt-1 whitespace-pre-wrap break-words">{a.note}</p>}
                {a.after && <ul className="mt-1 space-y-1">{(Object.keys(fieldLabels) as (keyof Fields)[]).filter(key => a.after?.[key] !== a.before?.[key]).map(key => <li key={key} className="whitespace-pre-wrap break-words">{fieldLabels[key]}: {a.before && `${display(key, a.before[key])} → `}{display(key, a.after![key])}</li>)}</ul>}
            </li>)}</ol>{!editing.activity.length && <Empty className="mt-2 px-3 py-4 text-xs text-muted-foreground"><EmptyDescription>No activity yet</EmptyDescription></Empty>}</details>}
        </div>
        <footer className="flex shrink-0 flex-wrap gap-2 border-t border-border p-4">
            <Button type="submit" aria-label={editing && !busy ? 'Save task' : undefined} disabled={busy || Boolean(changed) || conflict}>{busy ? 'Saving…' : editing ? 'Save' : 'Create task'}</Button>
            <Button type="button" variant="secondary" disabled={busy} onClick={close}>{editing ? 'Close' : 'Cancel'}</Button>
            <Button aria-label="Discard task changes" type="button" variant="ghost" disabled={busy} onClick={discard}>Discard</Button>
        </footer>
        </form>
    </section>
}
