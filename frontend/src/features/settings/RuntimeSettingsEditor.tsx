import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useRuntimeSettingsEditor } from './hooks/useRuntimeSettingsEditor'

export function RuntimeSettingsEditor() {
    const { saved, draft, setDraft, pending, error, message, setMessage, dirty, invalidRoots, save, discard } = useRuntimeSettingsEditor()
    return <Card className="gap-4 py-4 shadow-sm">
        <CardHeader className="px-4"><CardTitle className="text-sm">Runtime paths</CardTitle></CardHeader>
        <CardContent className="space-y-3 px-4">
            <p className="text-xs text-muted-foreground">Changes require restart. Leave a path empty to use its default. Command line and environment overrides take precedence.</p>
            {draft && saved && <>
                {(['runs_dir', 'flows_dir', 'ui_dir'] as const).map((key) => <Field key={key}>
                    <FieldLabel htmlFor={`runtime-${key}`}>{({ runs_dir: 'Runs directory', flows_dir: 'Flows directory', ui_dir: 'UI directory' })[key]}</FieldLabel>
                    <Input id={`runtime-${key}`} disabled={pending} value={draft[key] ?? ''} onChange={(event) => { setDraft({ ...draft, [key]: event.target.value || null }); setMessage('') }} />
                    <p className="text-xs text-muted-foreground">Effective: {saved.effective[key] ?? 'Not configured'} · Requires restart</p>
                </Field>)}
                <Field>
                    <FieldLabel htmlFor="runtime-project-roots">Project roots (one absolute path per line)</FieldLabel>
                    <textarea id="runtime-project-roots" className="min-h-20 rounded border p-2 text-sm" disabled={pending} aria-invalid={invalidRoots} aria-describedby={invalidRoots ? 'runtime-roots-error' : undefined} value={draft.project_roots.join('\n')} onChange={(event) => { setDraft({ ...draft, project_roots: event.target.value ? event.target.value.split('\n') : [] }); setMessage('') }} />
                    {invalidRoots && <p id="runtime-roots-error" role="alert">Each project root must be an absolute path.</p>}
                    <p className="text-xs text-muted-foreground">Effective: {saved.effective.project_roots.join(', ') || 'Default roots'} · Requires restart</p>
                </Field>
                <div className="flex gap-2">
                    <Button disabled={!dirty || pending || invalidRoots} onClick={() => void save()}>Save runtime settings</Button>
                    <Button variant="outline" disabled={pending || (!dirty && !error)} onClick={() => void discard()}>Discard runtime changes</Button>
                </div>
            </>}
            {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
            {message && <p role="status" className="text-sm">{message}</p>}
        </CardContent>
    </Card>
}
