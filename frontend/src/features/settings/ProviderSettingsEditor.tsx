import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useProviderSettingsEditor } from './hooks/useProviderSettingsEditor'
import { providers, providerFieldError, type ProviderConnection } from './services/executionSettings'

export function ProviderSettingsEditor() {
    const editor = useProviderSettingsEditor()
    return <Card>
        <CardHeader><CardTitle>Provider connections</CardTitle></CardHeader>
        <CardContent className="space-y-3">
            <p className="text-xs">Credentials stay in environment variables. Environment overrides take precedence; saved connections apply to new work.</p>
            {editor.draft && providers.map((provider) => <fieldset key={provider} disabled={editor.pending} className="space-y-2">
                <legend>{provider}</legend>
                {(['base_url', 'api_key_env', ...(provider === 'openai' ? ['organization', 'project'] : []), ...(provider === 'openrouter' ? ['http_referer', 'title'] : [])] as (keyof ProviderConnection)[]).map((key) => {
                    const value = editor.draft?.[provider]?.[key] ?? ''
                    const error = providerFieldError(key, value)
                    const id = `provider-${provider}-${key}`
                    return <Field key={key}><FieldLabel htmlFor={id}>{key === 'api_key_env' ? 'Credential environment variable' : key.replaceAll('_', ' ')}</FieldLabel>
                        <Input id={id} value={value} aria-invalid={!!error} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, [provider]: { ...draft[provider], [key]: event.target.value || null } }))} />
                        {error && <p role="alert">{error}</p>}
                        <p className="text-xs">Effective: {editor.saved?.effective[provider]?.[key] ?? 'Provider default'}{key === 'api_key_env' && ` · ${editor.saved?.credential_status[provider] ? 'Configured' : 'Missing'}`}</p>
                    </Field>
                })}
            </fieldset>)}
            <Button disabled={!editor.dirty || editor.pending || editor.invalid} onClick={() => void editor.save()}>Save provider connections</Button>
            <Button variant="outline" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard provider changes</Button>
            {editor.error && <p role="alert">{editor.error}</p>}{editor.message && <p role="status">{editor.message}</p>}
        </CardContent>
    </Card>
}
