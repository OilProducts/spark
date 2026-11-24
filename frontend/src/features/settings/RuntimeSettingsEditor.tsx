import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useRuntimeSettingsEditor } from './hooks/useRuntimeSettingsEditor'
import { SaveStatus } from './SaveStatus'

const isShadowed = (source?: string) => !!source && !['stored', 'built-in default'].includes(source)

export function RuntimeSettingsEditor() {
    const { saved, draft, setDraft, pending, error, message, setMessage, dirty, save, discard } = useRuntimeSettingsEditor()
    return <Card className="gap-4 py-4">
        <CardHeader className="px-4"><h2 className="text-base font-semibold">Runtime paths</h2></CardHeader>
        <CardContent className="space-y-3 px-4">
            {pending ? <p role="status">Updating…</p> : !saved && !error ? <p role="status">Loading settings…</p> : null}
            <p className="text-sm text-muted-foreground">Changes require restart. Leave a path empty to use its default.</p>
            {draft && saved && <>
                {(['runs_dir', 'ui_dir'] as const).map((key) => {
                    const fallback = ({ runs_dir: "Each project's private storage", ui_dir: 'Not configured' })[key]
                    const effective = saved.effective?.[key] ?? null
                    const stored = saved.stored?.[key] ?? null
                    const source = saved.sources?.[key]
                    const showEffective = !saved.effective || isShadowed(source) || effective !== stored
                    return <Field key={key}>
                        <FieldLabel htmlFor={`runtime-${key}`}>{({ runs_dir: 'Runs directory', ui_dir: 'UI directory' })[key]}</FieldLabel>
                        <Input id={`runtime-${key}`} disabled={pending} value={draft[key] ?? ''} placeholder={fallback} onChange={(event) => { setDraft({ ...draft, [key]: event.target.value || null }); setMessage('') }} />
                        {showEffective && <p className="text-xs text-muted-foreground">In use: {saved.effective ? effective ?? fallback : 'Unavailable'}{source ? ` · ${source}` : ''}</p>}
                    </Field>
                })}
                <Field>
                    <FieldLabel htmlFor="runtime-project-roots">Project roots (one absolute path per line)</FieldLabel>
                    <Textarea id="runtime-project-roots" className="min-h-20" disabled={pending} placeholder="Default roots" value={draft.project_roots.join('\n')} onChange={(event) => { setDraft({ ...draft, project_roots: event.target.value ? event.target.value.split('\n') : [] }); setMessage('') }} />
                    {(!saved.effective || isShadowed(saved.sources?.project_roots) || saved.effective.project_roots.join('\n') !== (saved.stored?.project_roots ?? []).join('\n')) && (
                        <p className="text-xs text-muted-foreground">In use: {saved.effective ? saved.effective.project_roots.join(', ') || 'Default roots' : 'Unavailable'}{saved.sources?.project_roots ? ` · ${saved.sources.project_roots}` : ''}</p>
                    )}
                </Field>
                <div className="flex flex-wrap gap-2">
                    <Button size="sm" aria-label="Save runtime paths" disabled={!dirty || pending} onClick={() => void save()}>Save</Button>
                    <Button size="sm" aria-label="Discard runtime paths changes" variant="ghost" disabled={pending || (!dirty && !error)} onClick={() => void discard()}>Discard</Button>
                </div>
            </>}
            {saved?.validation_errors?.length && saved.active_startup ? <p className="text-xs text-muted-foreground">Running paths: runs {saved.active_startup.runs_dir ?? "each project's private storage"}, UI {saved.active_startup.ui_dir ?? 'not configured'}, roots {saved.active_startup.project_roots.join(', ') || 'default roots'}.</p> : null}
            {!draft && saved?.repair_defaults && <Button variant="outline" disabled={pending} onClick={() => setDraft(saved!.repair_defaults!)}>Reset to defaults</Button>}
            {saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            <SaveStatus message={message} error={error} dirty={dirty} />
        </CardContent>
    </Card>
}
