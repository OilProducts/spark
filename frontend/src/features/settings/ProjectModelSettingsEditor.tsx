import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { useLlmProfiles } from '@/lib/useLlmProfiles'
import { getLlmSelectionOptions, splitLlmSelection } from '@/lib/llmSuggestions'
import { useModelSettingsEditor } from './hooks/useModelSettingsEditor'

export function ProjectModelSettingsEditor({ projectPath }: { projectPath: string }) {
    const editor = useModelSettingsEditor(projectPath)
    const profiles = useLlmProfiles()
    const selected = editor.draft?.llm_profile || editor.draft?.provider || ''
    const profile = profiles.find((entry) => entry.id === editor.draft?.llm_profile)
    const invalidModel = !!editor.draft && (profile ?
        (!!editor.draft.model && !profile.models.includes(editor.draft.model)) || (!editor.draft.model && !profile.default_model)
        : ['openrouter', 'litellm', 'openai_compatible'].includes(selected) && !editor.draft.model)
    return <Card className="gap-4 py-4 shadow-sm">
        <CardHeader className="px-4"><CardTitle className="text-sm">Project model defaults</CardTitle></CardHeader>
        <CardContent className="space-y-3 px-4">
            <p className="break-all text-xs text-muted-foreground">{projectPath}</p>
            <p className="text-xs">{editor.saved?.source === 'project' ? 'Project default' : 'Workspace default'}</p>
            <fieldset disabled={!editor.saved || editor.pending} className="space-y-3">
                <Label className="flex items-center gap-2 text-xs"><Switch disabled={!editor.draft && !editor.saved?.effective} checked={editor.draft !== null}
                    onCheckedChange={(checked) => editor.setDraft(checked && editor.saved?.effective ? { ...editor.saved.effective } : null)} />Override workspace model settings</Label>
                {editor.draft && <>
                    <Label className="block text-xs">Project provider or profile<NativeSelect value={selected} onChange={(event) => {
                        const selection = splitLlmSelection(event.target.value, profiles)
                        editor.setDraft({ provider: selection.llm_profile ? null : selection.llm_provider || 'codex', llm_profile: selection.llm_profile || null, model: null, reasoning_effort: null })
                    }}>{[...new Set([...getLlmSelectionOptions(profiles), selected])].filter(Boolean).map((value) => <option key={value} value={value}>{value}</option>)}</NativeSelect></Label>
                    <Label className="block text-xs">Project model<Input aria-invalid={!!invalidModel} value={editor.draft.model ?? ''}
                        placeholder="Use provider default" onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, model: event.target.value || null }))} /></Label>
                    {invalidModel && <p role="alert" className="text-xs text-destructive">Choose a compatible model for this provider or profile.</p>}
                    <Label className="block text-xs">Project reasoning effort<NativeSelect value={editor.draft.reasoning_effort ?? ''}
                        onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, reasoning_effort: event.target.value || null }))}>
                        <option value="">Use provider default</option>{['low', 'medium', 'high', 'xhigh'].map((value) => <option key={value} value={value}>{value}</option>)}
                    </NativeSelect></Label>
                </>}
            </fieldset>
            <div className="flex gap-2"><Button size="sm" disabled={!editor.dirty || editor.pending || !!invalidModel} onClick={() => void editor.save()}>Save project defaults</Button>
                <Button size="sm" variant="outline" disabled={!editor.saved || editor.pending} onClick={() => void editor.discard()}>Discard project changes</Button></div>
            {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            {editor.error && <p role="alert" className="text-xs text-destructive">{editor.error}</p>}
            {editor.message && <p role="status" className="text-xs">{editor.message}</p>}
        </CardContent>
    </Card>
}
