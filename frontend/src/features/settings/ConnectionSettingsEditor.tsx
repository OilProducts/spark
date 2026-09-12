import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useConnectionSettingsEditor } from './hooks/useConnectionSettingsEditor'

export function ConnectionSettingsEditor() {
    const editor = useConnectionSettingsEditor()
    return <Card>
        <CardHeader><CardTitle>Server and client connections</CardTitle></CardHeader>
        <CardContent className="space-y-3">
            <p className="text-xs">Server binding changes require restart. CLI and environment overrides take precedence. Desktop uses its native remote-access control and an automatically assigned port.</p>
            {editor.draft && editor.saved && <>
                <Field><FieldLabel htmlFor="connection-host">Server host</FieldLabel>
                    <Input id="connection-host" disabled={editor.pending} value={editor.draft.server_host ?? ''} aria-invalid={editor.invalidHost}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, server_host: event.target.value || null }))} />
                    {editor.invalidHost && <p role="alert">Enter an IP address or hostname.</p>}
                    <p className="text-xs">Effective: {editor.saved.effective.server_host ?? '127.0.0.1'} · Requires restart</p>
                </Field>
                <Field><FieldLabel htmlFor="connection-port">Server port</FieldLabel>
                    <Input id="connection-port" type="number" min={0} max={65535} disabled={editor.pending} value={editor.draft.server_port ?? ''} aria-invalid={editor.invalidPort}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, server_port: event.target.value === '' ? null : Number(event.target.value) }))} />
                    {editor.invalidPort && <p role="alert">Enter a whole number from 0 to 65535.</p>}
                    <p className="text-xs">Effective: {editor.saved.effective.server_port ?? 8000} · Requires restart · 0 assigns an available port</p>
                </Field>
                <Field><FieldLabel htmlFor="connection-target">Client API target</FieldLabel>
                    <Input id="connection-target" disabled={editor.pending} value={editor.draft.client_api_base_url ?? ''} aria-invalid={editor.invalidTarget}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, client_api_base_url: event.target.value || null }))} />
                    {editor.invalidTarget && <p role="alert">Enter an HTTP(S) URL without credentials, query or fragment.</p>}
                    <p className="text-xs">Effective: {editor.saved.effective.client_api_base_url ?? 'http://127.0.0.1:8000'} · Applies to new CLI commands</p>
                </Field>
                <div className="flex gap-2">
                    <Button disabled={!editor.dirty || editor.pending || editor.invalid} onClick={() => void editor.save()}>Save connection settings</Button>
                    <Button variant="outline" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard connection changes</Button>
                </div>
            </>}
            {editor.error && <p role="alert">{editor.error}</p>}
            {editor.message && <p role="status">{editor.message}</p>}
        </CardContent>
    </Card>
}
