import { isTauri } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useActiveEnvironment } from '@/lib/activeEnvironment'
import { useConnectionSettingsEditor } from './hooks/useConnectionSettingsEditor'
import { SaveStatus } from './SaveStatus'

const isShadowed = (source?: string) => !!source && !['stored', 'built-in default'].includes(source)

export function ConnectionSettingsEditor() {
    const editor = useConnectionSettingsEditor()
    const { activeEnvironmentId } = useActiveEnvironment()
    const hostEffective = editor.saved?.effective?.server_host ?? '127.0.0.1'
    const portEffective = editor.saved?.effective?.server_port ?? 8000
    const targetEffective = editor.saved?.effective?.client_api_base_url ?? 'http://127.0.0.1:8000'
    const showHost = !editor.saved?.effective || isShadowed(editor.saved.sources?.server_host) || (editor.saved.stored?.server_host ?? '127.0.0.1') !== hostEffective
    const showPort = !editor.saved?.effective || isShadowed(editor.saved.sources?.server_port) || (editor.saved.stored?.server_port ?? 8000) !== portEffective
    const showTarget = !editor.saved?.effective || isShadowed(editor.saved.sources?.client_api_base_url) || (editor.saved.stored?.client_api_base_url ?? 'http://127.0.0.1:8000') !== targetEffective
    return <Card>
        <CardHeader><h2 className="text-base font-semibold">Server and client connections</h2></CardHeader>
        <CardContent className="space-y-3">
        {editor.pending ? <p role="status">Updating…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
            {/* Only the desktop app manages its own server, and only on this machine; in a browser or on a linked environment this would describe another app. */}
            {isTauri() && activeEnvironmentId === null ? <p className="text-sm text-muted-foreground">The desktop app uses its own remote-access control and an automatically assigned port; these settings apply to a server started from the command line.</p> : null}
            {editor.saved?.running_server && <p className="text-xs text-muted-foreground">Running server: {editor.saved.running_server.server_host}:{editor.saved.running_server.server_port}</p>}
            {editor.draft && editor.saved && <>
                <Field><FieldLabel htmlFor="connection-host">Server host</FieldLabel>
                    <Input id="connection-host" disabled={editor.pending} value={editor.draft.server_host ?? ''} placeholder="127.0.0.1" aria-invalid={editor.invalidHost} aria-describedby={editor.invalidHost ? "connection-host-error" : undefined}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, server_host: event.target.value || null }))} />
                    {editor.invalidHost && <p id="connection-host-error" role="alert">Enter an IP address or hostname.</p>}
                    {showHost && <p className="text-xs text-muted-foreground">In use: {editor.saved.effective ? hostEffective : 'Unavailable'}{editor.saved.sources?.server_host ? ` · ${editor.saved.sources.server_host}` : ''}</p>}
                </Field>
                <Field><FieldLabel htmlFor="connection-port">Server port</FieldLabel>
                    <Input id="connection-port" className="w-40" type="number" min={0} max={65535} disabled={editor.pending} value={editor.draft.server_port ?? ''} placeholder="8000" aria-invalid={editor.invalidPort} aria-describedby={editor.invalidPort ? "connection-port-error" : undefined}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, server_port: event.target.value === '' ? null : Number(event.target.value) }))} />
                    {editor.invalidPort && <p id="connection-port-error" role="alert">Enter a whole number from 0 to 65535.</p>}
                    <p className="text-xs text-muted-foreground">
                        0 assigns an available port; host and port changes require restart.
                        {showPort ? ` In use: ${editor.saved.effective ? portEffective : 'Unavailable'}${editor.saved.sources?.server_port ? ` (${editor.saved.sources.server_port})` : ''}.` : ''}
                    </p>
                </Field>
                {editor.saved.client_config_dir && <p className="text-xs text-muted-foreground">Command-line settings folder: {editor.saved.client_config_dir}</p>}
                <Field><FieldLabel htmlFor="connection-target">Command-line server URL</FieldLabel>
                    <Input id="connection-target" disabled={editor.pending} value={editor.draft.client_api_base_url ?? ''} placeholder="http://127.0.0.1:8000" aria-invalid={editor.invalidTarget} aria-describedby={editor.invalidTarget ? "connection-target-error" : undefined}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, client_api_base_url: event.target.value || null }))} />
                    {editor.invalidTarget && <p id="connection-target-error" role="alert">Enter an HTTP(S) URL without credentials, query or fragment.</p>}
                    <p className="text-xs text-muted-foreground">
                        Where new spark commands connect.
                        {showTarget ? ` In use: ${editor.saved.effective ? targetEffective : 'Unavailable'}${editor.saved.sources?.client_api_base_url ? ` (${editor.saved.sources.client_api_base_url})` : ''}.` : ''}
                    </p>
                </Field>
                <div className="flex flex-wrap gap-2">
                    <Button size="sm" aria-label="Save server and client connections" disabled={!editor.dirty || editor.pending || editor.invalid} onClick={() => void editor.save()}>Save</Button>
                    <Button size="sm" aria-label="Discard server and client connections changes" variant="ghost" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard</Button>
                </div>
            </>}
            {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Reset to defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
        </CardContent>
    </Card>
}
