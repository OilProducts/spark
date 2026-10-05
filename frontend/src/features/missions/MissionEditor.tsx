import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import type { Mission, Fields, Playbook } from './MissionsPanel'

type Props = {
    editing: Mission | null; draft: Fields; latest?: Mission
    busy: boolean; conflict: boolean; error: string; unsaved: boolean; narrow: boolean; focusRequest: number
    setDraft: (fields: Fields) => void
    save: () => Promise<void>; archive: () => Promise<void>; close: () => void; discard: () => void; reconcile: () => void
}
type Core = 'title' | 'description' | 'archived'
const fieldLabels: Record<Core, string> = { title: 'Title', description: 'Objective', archived: 'Archived' }
function display(value: Fields[Core] | undefined) {
    if (typeof value === 'boolean') return value ? 'Yes' : 'No'
    return value || 'Empty'
}
export function MissionEditor({ editing, draft, latest, busy, conflict, error, unsaved, narrow, focusRequest, setDraft, save, archive, close, discard, reconcile }: Props) {
    const heading = useRef<HTMLHeadingElement>(null)
    const title = useRef<HTMLInputElement>(null)
    useEffect(() => { title.current?.focus({ preventScroll: true }) }, [focusRequest])
    const changed = latest && latest.revision > (editing?.revision ?? 0)
    const [playbooks, setPlaybooks] = useState<Playbook[]>([])
    const started = Boolean(editing?.started_at)
    useEffect(() => {
        if (started) return
        let disposed = false
        void fetch('/workspace/api/playbooks').then(response => response.ok ? response.json() : []).then(value => { if (!disposed && Array.isArray(value)) setPlaybooks(value) }).catch(() => {})
        return () => { disposed = true }
    }, [started])
    return <section aria-label="Mission details" className={`flex min-h-0 min-w-0 flex-col rounded-md border border-border ${narrow ? 'w-full' : 'w-[28rem] shrink-0'}`} onKeyDown={e => {
        if (e.key === 'Escape' && !e.defaultPrevented && !e.nativeEvent.isComposing && !busy) { e.stopPropagation(); close() }
    }}>
        <header className="shrink-0 border-b border-border p-4">
            <h2 ref={heading} tabIndex={-1} className="text-lg font-light break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{editing?.fields.title ?? 'New mission'}</h2>
            {unsaved && <p className="mt-1 text-xs text-muted-foreground">Unsaved changes</p>}
        </header>
        <form className="flex min-h-0 flex-1 flex-col" onSubmit={e => { e.preventDefault(); void save() }}>
        <div className="min-h-0 flex-1 overflow-y-auto p-4 space-y-4 text-sm">
            {error && <InlineError>{error}</InlineError>}
            {conflict && !changed && <p role="status">This mission changed. Refresh to see the latest version; your draft is kept.</p>}
            {changed && <div role="status" className="space-y-3 rounded-md border border-border p-3">
                <p>A newer version exists. Your edits are kept and win when you reconcile.</p>
                <dl className="space-y-2">{(Object.keys(fieldLabels) as Core[]).filter(key => latest.fields[key] !== editing?.fields[key]).map(key => <div key={key}><dt className="font-medium">{fieldLabels[key]}</dt><dd className="whitespace-pre-wrap break-words text-muted-foreground">{display(latest.fields[key])}</dd></div>)}</dl>
                <Button variant="secondary" type="button" disabled={busy} onClick={reconcile}>Reconcile with latest revision</Button>
            </div>}
            <fieldset disabled={busy} className="grid min-w-0 gap-4">
                <Label className="grid gap-2">Title<Input ref={title} required value={draft.title} onChange={e => setDraft({ ...draft, title: e.target.value })} /></Label>
                <Label className="grid gap-2">Objective<Textarea className="min-h-32" value={draft.description} onChange={e => setDraft({ ...draft, description: e.target.value })} /></Label>
                {!started && <Label className="grid gap-2">Playbook<NativeSelect className="w-full" value={draft.playbook ?? ''} onChange={e => setDraft({ ...draft, playbook: e.target.value || null })}>
                    <NativeSelectOption value="">None</NativeSelectOption>
                    {draft.playbook && !playbooks.some(p => p.name === draft.playbook) && <NativeSelectOption value={draft.playbook}>{draft.playbook}</NativeSelectOption>}
                    {playbooks.map(p => <NativeSelectOption key={p.name} value={p.name} title={p.description}>{p.title || p.name}</NativeSelectOption>)}
                </NativeSelect></Label>}
            </fieldset>
            {editing && <div className="border-t border-border pt-4"><Button type="button" variant="secondary" disabled={busy || Boolean(changed) || conflict} onClick={() => void archive()}>{editing.fields.archived ? 'Restore mission' : 'Archive mission'}</Button></div>}
            {editing && <details className="border-t border-border pt-4"><summary className="cursor-pointer rounded-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">Activity</summary><ol className="mt-2 space-y-3">{(latest ?? editing).activity.map(a => <li key={a.revision} className="border-b border-border pb-3">
                <p className="text-xs text-muted-foreground">{a.actor === 'assistant' ? 'Assistant' : 'Human'} · <time>{a.at}</time></p>
                {a.note && <p className="mt-1 whitespace-pre-wrap break-words">{a.note}</p>}
                {a.after && <ul className="mt-1 space-y-1">{(Object.keys(fieldLabels) as Core[]).filter(key => a.after?.[key] !== a.before?.[key]).map(key => <li key={key} className="whitespace-pre-wrap break-words">{fieldLabels[key]}: {a.before && `${display(a.before[key])} → `}{display(a.after![key])}</li>)}</ul>}
            </li>)}</ol>{!editing.activity.length && <Empty className="mt-2 px-3 py-4 text-xs text-muted-foreground"><EmptyDescription>No activity yet</EmptyDescription></Empty>}</details>}
        </div>
        <footer className="flex shrink-0 flex-wrap gap-2 border-t border-border p-4">
            <Button type="submit" aria-label={editing && !busy ? 'Save mission' : undefined} disabled={busy || Boolean(changed) || conflict}>{busy ? 'Saving…' : editing ? 'Save' : 'Create mission'}</Button>
            <Button type="button" variant="secondary" disabled={busy} onClick={close}>{editing ? 'Close' : 'Cancel'}</Button>
            <Button aria-label="Discard mission changes" type="button" variant="ghost" disabled={busy} onClick={discard}>Discard</Button>
        </footer>
        </form>
    </section>
}
