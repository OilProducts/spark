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
            {editor.saved?.running_server && <p className="text-xs">Running server: {editor.saved.running_server.server_host}:{editor.saved.running_server.server_port}</p>}
            {editor.draft && editor.saved && <>
                <Field><FieldLabel htmlFor="connection-host">Server host</FieldLabel>
                    <Input id="connection-host" disabled={editor.pending} value={editor.draft.server_host ?? ''} aria-invalid={editor.invalidHost}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, server_host: event.target.value || null }))} />
                    {editor.invalidHost && <p role="alert">Enter an IP address or hostname.</p>}
                    <p className="text-xs">Effective: {editor.saved.effective ? editor.saved.effective.server_host ?? '127.0.0.1' : 'Unavailable'} · {editor.saved.sources?.server_host} · Requires restart</p>
                </Field>
                <Field><FieldLabel htmlFor="connection-port">Server port</FieldLabel>
                    <Input id="connection-port" type="number" min={0} max={65535} disabled={editor.pending} value={editor.draft.server_port ?? ''} aria-invalid={editor.invalidPort}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, server_port: event.target.value === '' ? null : Number(event.target.value) }))} />
                    {editor.invalidPort && <p role="alert">Enter a whole number from 0 to 65535.</p>}
                    <p className="text-xs">Effective: {editor.saved.effective ? editor.saved.effective.server_port ?? 8000 : 'Unavailable'} · {editor.saved.sources?.server_port} · Requires restart · 0 assigns an available port</p>
                </Field>
                {editor.saved.client_config_dir && <p className="text-xs">New CLI configuration directory: {editor.saved.client_config_dir}</p>}
                <Field><FieldLabel htmlFor="connection-target">Client API target</FieldLabel>
                    <Input id="connection-target" disabled={editor.pending} value={editor.draft.client_api_base_url ?? ''} aria-invalid={editor.invalidTarget}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, client_api_base_url: event.target.value || null }))} />
                    {editor.invalidTarget && <p role="alert">Enter an HTTP(S) URL without credentials, query or fragment.</p>}
                    <p className="text-xs">Effective: {editor.saved.effective ? editor.saved.effective.client_api_base_url ?? 'http://127.0.0.1:8000' : 'Unavailable'} · {editor.saved.sources?.client_api_base_url} · Applies to new CLI commands</p>
                </Field>
                <div className="flex gap-2">
                    <Button disabled={!editor.dirty || editor.pending || editor.invalid} onClick={() => void editor.save()}>Save connection settings</Button>
                    <Button variant="outline" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard connection changes</Button>
                </div>
            </>}
            {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            {editor.error && <p role="alert">{editor.error}</p>}
            {editor.message && <p role="status">{editor.message}</p>}
        </CardContent>
    </Card>
}
