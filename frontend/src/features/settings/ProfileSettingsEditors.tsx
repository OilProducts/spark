import { useId, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { parseExecutionProfiles, parseLlmProfiles, type LlmProfileSettings, type ExecutionProfileSettings } from './services/profileSettings'
import { useProfileSettingsEditor } from './hooks/useProfileSettingsEditor'

const splitLines = (value: string) => value.split('\n').map((entry) => entry.trim()).filter(Boolean)
const duplicateIds = (profiles: { id: string }[]) => profiles.some((profile, index) => !profile.id.trim() || profile.id !== profile.id.trim() || profiles.findIndex((entry) => entry.id === profile.id) !== index)
const validEndpoint = (value: string) => {
    try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password && !url.search && !url.hash }
    catch { return false }
}

export function LlmProfilesEditor() {
    const id = useId()
    const editor = useProfileSettingsEditor('llm_profiles', parseLlmProfiles)
    const profiles = editor.draft ?? []
    const invalid = duplicateIds(profiles) || profiles.some((profile) => !validEndpoint(profile.base_url) || !profile.models.length
        || (profile.default_model && !profile.models.includes(profile.default_model))
        || (profile.api_key_env && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(profile.api_key_env)))
    const change = (index: number, patch: Partial<LlmProfileSettings>) => editor.setDraft(profiles.map((profile, at) => at === index ? { ...profile, ...patch } : profile))
    return <Card><CardHeader><h3 className="text-sm font-semibold">LLM profiles</h3></CardHeader><CardContent className="space-y-3">
        {editor.pending ? <p role="status">Saving or reloading settings…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
        <p className="text-xs text-muted-foreground">Workspace profiles. Credentials remain in environment variables; only their names are saved.</p>
        <fieldset disabled={editor.pending || !editor.saved} className="space-y-4">
            {profiles.map((profile, index) => {
                const errors = {
                    id: !profile.id.trim() || profile.id !== profile.id.trim() || profiles.filter((p) => p.id === profile.id).length > 1 ? 'Enter a unique, nonempty profile ID without surrounding whitespace.' : '',
                    endpoint: !validEndpoint(profile.base_url) ? 'Use an HTTP(S) endpoint without embedded credentials, query parameters or fragments.' : '',
                    credential: profile.api_key_env && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(profile.api_key_env) ? 'Enter an environment variable name, not a credential value.' : '',
                    models: !profile.models.length || profile.models.some((m) => !m.trim()) ? 'Enter at least one nonempty model name.' : '',
                    default: profile.default_model && !profile.models.includes(profile.default_model) ? 'Choose a default from this profile’s models.' : '',
                }
                const errorId = `${id}-${index}-errors`
                return <details key={index} ref={(node) => { if (node && Object.values(errors).some(Boolean)) node.open = true }} className="space-y-2 rounded border p-3">
                <summary>{profile.label || profile.id || `LLM profile ${index + 1}`} · {editor.saved?.credential_status?.[profile.id] ?? 'unsaved'}</summary>
                <label className="block">Profile ID<Input aria-label={`LLM profile ${index + 1} ID`} aria-invalid={!!errors.id} aria-describedby={errors.id ? errorId : undefined} value={profile.id} onChange={(event) => change(index, { id: event.target.value })} /></label>
                <label className="block">Label<Input value={profile.label ?? ''} onChange={(event) => change(index, { label: event.target.value || undefined })} /></label>
                <p className="text-xs">Provider: openai_compatible</p>
                <label className="block">Endpoint<Input aria-invalid={!!errors.endpoint} aria-describedby={errors.endpoint ? errorId : undefined} value={profile.base_url} onChange={(event) => change(index, { base_url: event.target.value })} /></label>
                <label className="block">Credential environment variable<Input aria-invalid={!!errors.credential} aria-describedby={errors.credential ? errorId : undefined} value={profile.api_key_env ?? ''} onChange={(event) => change(index, { api_key_env: event.target.value || undefined })} /></label>
                <p className="text-xs">Saved credential status: {editor.saved?.credential_status?.[profile.id] ?? 'unsaved'}</p>
                <label className="block">Models (one per line)<textarea aria-invalid={!!errors.models} aria-describedby={errors.models ? errorId : undefined} className="block w-full rounded border p-2" value={profile.models.join('\n')} onChange={(event) => change(index, { models: event.target.value.split('\n') })} /></label>
                <label className="block">Default model<NativeSelect aria-label="Default model" aria-invalid={!!errors.default} aria-describedby={errors.default ? errorId : undefined} value={profile.default_model ?? ''} onChange={(event) => change(index, { default_model: event.target.value || undefined })}>
                    <option value="">Require an explicit model</option>{profile.models.map((model, at) => <option key={at} value={model}>{model}</option>)}
                </NativeSelect></label>
                <div id={errorId}>{Object.entries(errors).filter(([, error]) => error).map(([key, error]) => <p role="alert" key={key}>{error}</p>)}</div>
                <Button variant="outline" onClick={() => editor.setDraft(profiles.filter((_, at) => at !== index))}>Delete LLM profile {profile.id}</Button>
            </details>})}
            <Button variant="outline" onClick={() => editor.setDraft([...profiles, { id: '', provider: 'openai_compatible', base_url: '', models: [] }])}>Add LLM profile</Button>
        </fieldset>
        {duplicateIds(profiles) && <p role="alert">Enter unique, nonempty profile IDs without surrounding whitespace.</p>}
        <div className="flex flex-wrap gap-2"><Button disabled={!editor.dirty || editor.pending || !!invalid || profiles.some((p) => p.models.some((m) => !m.trim()))} onClick={() => void editor.save()}>Save LLM profiles</Button>
            <Button variant="outline" disabled={editor.pending || !editor.saved} onClick={() => void editor.discard()}>Discard LLM profile changes</Button></div>
        {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            {editor.error && <p role="alert">{editor.error}</p>}{editor.message && <p role="status">{editor.message}</p>}
    </CardContent></Card>
}

function ExecutionProfileFields({ profile, index, change, remove, onValidity, deletionBlocked, invalidId, savedStatus }: {
    savedStatus: string; deletionBlocked: boolean; invalidId: boolean; profile: ExecutionProfileSettings; index: number; change: (patch: Partial<ExecutionProfileSettings>) => void; remove: () => void; onValidity: (invalid: boolean) => void
}) {
    const id = useId()
    const [metadata, setMetadata] = useState(JSON.stringify(profile.metadata, null, 2))
    const [error, setError] = useState('')
    const mounts = profile.metadata['container.mounts']
    const [mountText, setMountText] = useState(Array.isArray(mounts) ? mounts.join('\n') : '')
    return <details ref={(node) => { if (node && (invalidId || !profile.label.trim() || (profile.mode === 'local_container' && !profile.image?.trim()) || !!error)) node.open = true }} className="space-y-2 rounded border p-3"><summary>{profile.label || profile.id || `Execution profile ${index + 1}`} · {savedStatus}</summary>
        <label className="block">Profile ID<Input aria-invalid={invalidId} aria-describedby={invalidId ? `${id}-profile-error` : undefined} value={profile.id} onChange={(event) => change({ id: event.target.value })} /></label>
        <label className="block">Label<Input aria-invalid={!profile.label.trim()} aria-describedby={!profile.label.trim() ? `${id}-profile-error` : undefined} value={profile.label} onChange={(event) => change({ label: event.target.value })} /></label>
        {(invalidId || !profile.label.trim()) && <p id={`${id}-profile-error`} role="alert">Enter a unique profile ID without surrounding whitespace and a nonempty label.</p>}
        <label className="block">Mode<NativeSelect aria-label="Mode" value={profile.mode} onChange={(event) => change({ mode: event.target.value as ExecutionProfileSettings['mode'] })}><option value="native">Native</option><option value="local_container">Local container</option></NativeSelect></label>
        <label className="block"><input type="checkbox" checked={profile.enabled} onChange={(event) => change({ enabled: event.target.checked })} /> Enabled</label>
        <div hidden={profile.mode !== 'local_container'}>
        <label className="block">Container image<Input aria-invalid={profile.mode === 'local_container' && !profile.image?.trim()} aria-describedby={profile.mode === 'local_container' && !profile.image?.trim() ? `${id}-image-error` : undefined} value={profile.image ?? ''} onChange={(event) => change({ image: event.target.value || undefined })} /></label>
        {profile.mode === 'local_container' && !profile.image?.trim() && <p id={`${id}-image-error`} role="alert">A container image is required.</p>}
        <label className="block">Mounts (host:container[:options], one per line)<textarea className="block w-full rounded border p-2" value={mountText} onChange={(event) => {
            setMountText(event.target.value)
            const next = { ...profile.metadata, 'container.mounts': splitLines(event.target.value) }
            change({ metadata: next }); setMetadata(JSON.stringify(next, null, 2)); setError(''); onValidity(false)
        }} /></label>
        </div>
        <details ref={(node) => { if (node && (!!error)) node.open = true }}><summary>Advanced</summary>
        <label className="block">Capabilities (one per line)<textarea className="block w-full rounded border p-2" value={profile.capabilities.join('\n')} onChange={(event) => change({ capabilities: event.target.value.split('\n') })} onBlur={(event) => change({ capabilities: splitLines(event.target.value) })} /></label>
        <label className="block">Metadata (JSON object)<textarea className="block w-full rounded border p-2 font-mono" aria-invalid={!!error} aria-describedby={error ? `${id}-metadata-error` : undefined} value={metadata} onChange={(event) => {
            setMetadata(event.target.value)
            try {
                const value: unknown = JSON.parse(event.target.value)
                if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Expected an object')
                const next = value as Record<string, unknown>
                change({ metadata: next }); setMountText(Array.isArray(next['container.mounts']) ? next['container.mounts'].join('\n') : ''); setError(''); onValidity(false)
            } catch { setError('Enter a valid JSON object.'); onValidity(true) }
        }} /></label>
        {error && <p id={`${id}-metadata-error`} role="alert">{error}</p>}
        </details>
        <Button variant="outline" disabled={deletionBlocked} aria-describedby={deletionBlocked ? "execution-delete-help" : undefined} onClick={remove}>Delete execution profile {profile.id}</Button>
    </details>
}

export function ExecutionProfilesEditor() {
    const [invalidMetadata, setInvalidMetadata] = useState<Record<number, boolean>>({})
    const editor = useProfileSettingsEditor('execution_profiles', parseExecutionProfiles, Object.values(invalidMetadata).some(Boolean))
    const [generation, setGeneration] = useState(0)
    const profiles = editor.draft?.profiles ?? []
    const invalid = duplicateIds(profiles) || profiles.some((profile) => !profile.label.trim() || (profile.mode === 'local_container' && !profile.image?.trim())) || Object.values(invalidMetadata).some(Boolean)
    const change = (index: number, patch: Partial<ExecutionProfileSettings>) => editor.setDraft((draft) => draft && ({ ...draft, profiles: draft.profiles.map((profile, at) => at === index ? { ...profile, ...patch } : profile) }))
    return <Card><CardHeader><h3 className="text-sm font-semibold">Execution profiles</h3></CardHeader><CardContent className="space-y-3">
        {editor.pending ? <p role="status">Saving or reloading settings…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
        <p className="text-xs text-muted-foreground">Workspace defaults. Saved changes apply to new work; active runs retain their captured profiles.</p>
        <fieldset disabled={editor.pending || !editor.saved} className="space-y-3">
            <label className="block">Default execution profile<NativeSelect aria-label="Default execution profile" value={editor.draft?.default_execution_profile_id ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, default_execution_profile_id: event.target.value || null }))}>
                <option value="">Runtime default</option>{profiles.filter((profile) => profile.enabled).map((profile, index) => <option key={index} value={profile.id}>{profile.label || profile.id}</option>)}
            </NativeSelect></label>
            {profiles.map((profile, index) => <ExecutionProfileFields key={`${editor.saved?.revision}-${generation}-${index}`} profile={profile} index={index} savedStatus={editor.saved?.stored?.profiles.find((saved) => saved.id === profile.id)?.enabled === true ? 'Saved: enabled' : editor.saved?.stored?.profiles.some((saved) => saved.id === profile.id) ? 'Saved: disabled' : 'Unsaved'} invalidId={!profile.id.trim() || profile.id !== profile.id.trim() || profiles.filter((p) => p.id === profile.id).length > 1} deletionBlocked={Object.values(invalidMetadata).some(Boolean)} change={(patch) => change(index, patch)}
                onValidity={(invalid) => setInvalidMetadata((value) => ({ ...value, [index]: invalid }))}
                remove={() => { if (Object.values(invalidMetadata).some(Boolean)) return; editor.setDraft((draft) => draft && ({ ...draft, profiles: profiles.filter((_, at) => at !== index) })); setGeneration((value) => value + 1); setInvalidMetadata({}) }} />)}
            <Button variant="outline" onClick={() => editor.setDraft((draft) => draft && ({ ...draft, profiles: [...profiles, { id: '', label: '', mode: 'native', enabled: true, capabilities: [], metadata: {} }] }))}>Add execution profile</Button>
        </fieldset>
        {Object.values(invalidMetadata).some(Boolean) && <p id="execution-delete-help" role="alert">Fix invalid metadata JSON before deleting any execution profile. This protects local drafts.</p>}
        {duplicateIds(profiles) && <p role="alert">Enter unique, nonempty profile IDs without surrounding whitespace.</p>}
        <div className="flex flex-wrap gap-2"><Button disabled={!editor.dirty || editor.pending || invalid} onClick={() => void editor.save()}>Save execution profiles</Button>
            <Button variant="outline" disabled={editor.pending || !editor.saved} onClick={() => { void editor.discard().then((discarded) => { if (discarded) { setGeneration((value) => value + 1); setInvalidMetadata({}) } }) }}>Discard execution profile changes</Button></div>
        {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            {editor.error && <p role="alert">{editor.error}</p>}{editor.message && <p role="status">{editor.message}</p>}
    </CardContent></Card>
}
