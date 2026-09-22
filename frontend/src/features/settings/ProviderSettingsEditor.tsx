import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { useProviderSettingsEditor } from './hooks/useProviderSettingsEditor'
import { SaveStatus } from './SaveStatus'
import { providers, providerFieldError, type ProviderConnection } from './services/executionSettings'

const providerNames = { openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Gemini', openrouter: 'OpenRouter', litellm: 'LiteLLM', openai_compatible: 'OpenAI compatible' }

export function ProviderSettingsEditor() {
    const editor = useProviderSettingsEditor()
    return <Card>
        <CardHeader><h3 className="text-base font-semibold">Provider connections</h3></CardHeader>
        <CardContent className="space-y-3">
        {editor.pending ? <p role="status">Saving or reloading settings…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
            <p className="text-xs">Credentials stay in environment variables. Environment overrides take precedence; saved connections apply to new work.</p>
            {editor.draft && providers.map((provider) => <details key={provider} ref={(node) => { if (node && (Object.entries(editor.draft?.[provider] ?? {}).some(([key, value]) => !!providerFieldError(key as keyof ProviderConnection, value ?? '')))) node.open = true }}>
                <summary className="cursor-pointer">{providerNames[provider]} · {editor.saved?.credential_status[provider] ? 'Configured' : 'Missing credentials'}</summary>
                <fieldset disabled={editor.pending} className="space-y-2">
                <legend className="sr-only">{providerNames[provider]}</legend>
                {(['base_url', 'api_key_env', ...(provider === 'openai' ? ['organization', 'project'] : []), ...(provider === 'openrouter' ? ['http_referer', 'title'] : [])] as (keyof ProviderConnection)[]).map((key) => {
                    const value = editor.draft?.[provider]?.[key] ?? ''
                    const error = providerFieldError(key, value)
                    const id = `provider-${provider}-${key}`
                    return <Field key={key}><FieldLabel htmlFor={id}>{({ base_url: 'Base URL', api_key_env: 'Credential environment variable', organization: 'Organization', project: 'Project', http_referer: 'HTTP referrer', title: 'Application title' })[key]}</FieldLabel>
                        <Input id={id} value={value} aria-invalid={!!error} aria-describedby={error ? `${id}-error` : undefined} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, [provider]: { ...draft[provider], [key]: event.target.value || null } }))} />
                        {error && <p id={`${id}-error`} role="alert">{error}</p>}
                        <p className="text-xs">Effective: {editor.saved?.effective ? editor.saved.effective[provider]?.[key] ?? 'Provider default' : 'Unavailable'} · {editor.saved?.sources?.[`${provider}.${key}`]}{key === 'api_key_env' && ` · ${editor.saved?.credential_status[provider] ? 'Configured' : 'Missing'}`}</p>
                    </Field>
                })}
            </fieldset></details>)}
            <div className="flex flex-wrap gap-2">
            <Button disabled={!editor.dirty || editor.pending || editor.invalid} onClick={() => void editor.save()}>Save</Button>
            <Button variant="outline" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard</Button>
            </div>
            {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
        </CardContent>
    </Card>
}
