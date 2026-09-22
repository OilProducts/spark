import { isModelSelectionValid } from '@/lib/llmSuggestions'
import { ModelSettingsFields } from './ModelSettingsFields'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { useLlmProfiles } from '@/lib/useLlmProfiles'
import { useModelSettingsEditor } from './hooks/useModelSettingsEditor'
import { SaveStatus } from './SaveStatus'

export function ProjectModelSettingsEditor({ projectPath }: { projectPath: string }) {
    const editor = useModelSettingsEditor(projectPath)
    const profiles = useLlmProfiles()
    const invalidModel = !!editor.draft && !isModelSelectionValid(editor.draft.llm_profile || editor.draft.provider || '', editor.draft.model, profiles)
    return <Card className="gap-4 py-4 shadow-sm">
        <CardHeader className="px-4"><h3 className="text-base font-semibold">Project model defaults</h3></CardHeader>
        <CardContent className="space-y-3 px-4">
            {editor.pending ? <p role="status">Saving or reloading settings…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
            <p className="break-all text-xs text-muted-foreground">{projectPath}</p>
            <p className="text-xs">Active-project overrides. Unsaved edits do not change the saved effective values below.</p>
            <p className="text-xs">Saved effective: {editor.saved?.effective ? <>
                Provider or profile: {profiles.find((p) => p.id === editor.saved?.effective?.llm_profile)?.label} {editor.saved.effective.llm_profile ?? editor.saved.effective.provider ?? 'Provider default'} ·
                Model: {editor.saved.effective.model ?? profiles.find((p) => p.id === editor.saved?.effective?.llm_profile)?.default_model ?? 'Provider default'} ·
                Reasoning effort: {editor.saved.effective.reasoning_effort ?? 'Provider default'}
            </> : 'Unavailable'}</p>
            <p className="text-xs">{editor.saved?.source === 'project' ? 'Project default' : 'Workspace default'}</p>
            <fieldset disabled={!editor.saved || editor.pending} className="space-y-3">
                <Label className="flex items-center gap-2 text-sm"><Switch disabled={!editor.draft && !editor.saved?.effective} checked={editor.draft !== null}
                    onCheckedChange={(checked) => editor.setDraft(checked && editor.saved?.effective ? { ...editor.saved.effective } : null)} />Override workspace model settings</Label>
                {editor.draft && <ModelSettingsFields profiles={profiles} models={editor} activeProjectPath={projectPath} invalidModel={!!invalidModel} />}
            </fieldset>
            <div className="flex flex-wrap gap-2"><Button size="sm" disabled={!editor.dirty || editor.pending || !!invalidModel} onClick={() => void editor.save()}>Save</Button>
                <Button size="sm" variant="outline" disabled={!editor.saved || editor.pending} onClick={() => void editor.discard()}>Discard</Button></div>
            {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
        </CardContent>
    </Card>
}
