import { useEffect, useRef } from 'react'
import { Button } from '@/components/ui/button'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { InlineError } from '@/components/app/inline-error'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { labels, stages, type Stage, type Task } from './TasksPanel'

type Props = {
    task: Task; busy: boolean; error: string; narrow: boolean; focusRequest: number
    edit: () => void; close: () => void; changeStage: (stage: Stage) => Promise<void>
}
export function TaskDetail({ task, busy, error, narrow, focusRequest, edit, close, changeStage }: Props) {
    const heading = useRef<HTMLHeadingElement>(null)
    useEffect(() => { heading.current?.focus({ preventScroll: true }) }, [task.id, focusRequest])
    return <section aria-label="Task details" className={`flex min-h-0 min-w-0 flex-col rounded-md border border-border bg-card ${narrow ? 'w-full' : 'w-[28rem] shrink-0'}`} onKeyDown={e => {
        if (e.key === 'Escape' && !e.defaultPrevented && !e.nativeEvent.isComposing && !busy) { e.stopPropagation(); close() }
    }}>
        <div className="min-h-0 flex-1 overflow-y-auto p-4 space-y-4 text-sm">
            <h2 ref={heading} tabIndex={-1} className="text-lg font-semibold whitespace-pre-wrap break-words focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{task.fields.title}</h2>
            {task.fields.archived && <span className="inline-block rounded border border-border px-2 py-1 text-xs text-muted-foreground">Archived</span>}
            <Label className="grid gap-2">Stage<NativeSelect disabled={busy} value={task.fields.stage} onChange={e => void changeStage(e.target.value as Stage)}>{stages.map((stage, i) => <NativeSelectOption key={stage} value={stage}>{labels[i]}</NativeSelectOption>)}</NativeSelect></Label>
            {busy && <p role="status">Saving…</p>}
            {error && <InlineError>{error}</InlineError>}
            <p className="whitespace-pre-wrap break-words leading-relaxed">{task.fields.description || 'No description'}</p>
            <details className="border-t border-border pt-4"><summary className="cursor-pointer rounded-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">Activity and details</summary>
                <dl className="mt-3 text-xs text-muted-foreground"><dt>Task ID</dt><dd className="break-all">{task.id}</dd><dt className="mt-2">Revision</dt><dd>{task.revision}</dd></dl>
                <ol className="mt-3 space-y-3">{task.activity.map(entry => <li key={entry.revision}><p className="text-xs text-muted-foreground">{entry.actor} · <time>{entry.at}</time> · Revision {entry.revision}</p><p className="whitespace-pre-wrap break-words">{entry.note}</p></li>)}</ol>
                {!task.activity.length && <Empty className="mt-2 px-3 py-4 text-xs text-muted-foreground"><EmptyDescription>No activity yet</EmptyDescription></Empty>}
            </details>
        </div>
        <footer className="flex shrink-0 gap-2 border-t border-border p-4"><Button disabled={busy} onClick={edit}>Edit</Button><Button variant="secondary" disabled={busy} onClick={close}>Close</Button></footer>
    </section>
}
