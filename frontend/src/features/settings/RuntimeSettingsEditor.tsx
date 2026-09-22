import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useRuntimeSettingsEditor } from './hooks/useRuntimeSettingsEditor'

export function RuntimeSettingsEditor() {
    const { saved, draft, setDraft, pending, error, message, setMessage, dirty, invalidRoots, save, discard } = useRuntimeSettingsEditor()
    return <Card className="gap-4 py-4 shadow-sm">
        <CardHeader className="px-4"><h3 className="text-base font-semibold">Runtime paths</h3></CardHeader>
        <CardContent className="space-y-3 px-4">
            {pending ? <p role="status">Saving or reloading settings…</p> : !saved && !error ? <p role="status">Loading settings…</p> : null}
            <p className="text-xs text-muted-foreground">Changes require restart. Leave a path empty to use its default. Command line and environment overrides take precedence.</p>
            {draft && saved && <>
                {(['runs_dir', 'flows_dir', 'ui_dir'] as const).map((key) => <Field key={key}>
                    <FieldLabel htmlFor={`runtime-${key}`}>{({ runs_dir: 'Runs directory', flows_dir: 'Flows directory', ui_dir: 'UI directory' })[key]}</FieldLabel>
                    <Input id={`runtime-${key}`} disabled={pending} value={draft[key] ?? ''} onChange={(event) => { setDraft({ ...draft, [key]: event.target.value || null }); setMessage('') }} />
                    <p className="text-xs text-muted-foreground">Effective: {saved.effective ? saved.effective[key] ?? 'Not configured' : 'Unavailable'} · {saved.sources?.[key]} · Requires restart</p>
                </Field>)}
                <Field>
                    <FieldLabel htmlFor="runtime-project-roots">Project roots (one absolute path per line)</FieldLabel>
                    <textarea id="runtime-project-roots" className="min-h-20 rounded border p-2 text-sm" disabled={pending} aria-invalid={invalidRoots} aria-describedby={invalidRoots ? 'runtime-roots-error' : undefined} value={draft.project_roots.join('\n')} onChange={(event) => { setDraft({ ...draft, project_roots: event.target.value ? event.target.value.split('\n') : [] }); setMessage('') }} />
                    {invalidRoots && <p id="runtime-roots-error" role="alert">Each project root must be an absolute path.</p>}
                    <p className="text-xs text-muted-foreground">Effective: {saved.effective ? saved.effective.project_roots.join(', ') || 'Default roots' : 'Unavailable'} · {saved.sources?.project_roots} · Requires restart</p>
                </Field>
                <div className="flex flex-wrap gap-2">
                    <Button disabled={!dirty || pending || invalidRoots} onClick={() => void save()}>Save runtime settings</Button>
                    <Button variant="outline" disabled={pending || (!dirty && !error)} onClick={() => void discard()}>Discard runtime changes</Button>
                </div>
            </>}
            {saved?.validation_errors?.length && saved.active_startup ? <p className="text-xs">Running paths: flows {saved.active_startup.flows_dir}, runs {saved.active_startup.runs_dir}, UI {saved.active_startup.ui_dir ?? 'not configured'}, roots {saved.active_startup.project_roots.join(', ') || 'default roots'}.</p> : null}
            {!draft && saved?.repair_defaults && <Button variant="outline" disabled={pending} onClick={() => setDraft(saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
            {message && <p role="status" className="text-sm">{message}</p>}
        </CardContent>
    </Card>
}
