import { isModelSelectionValid } from '@/lib/llmSuggestions'
import { ModelChooser } from '@/components/model-chooser/ModelChooser'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { useLlmProfiles } from '@/lib/useLlmProfiles'
import { useModelOptions } from '@/components/model-chooser/useModelOptions'
import { useModelSettingsEditor } from './hooks/useModelSettingsEditor'
import { SaveStatus } from './SaveStatus'

const codexDefault = { provider: 'codex', llm_profile: null, model: null, reasoning_effort: null }
// Small, fast models for the first provider available: Codex, then Claude Code.
const UTILITY_DEFAULTS = [
    { ...codexDefault, model: 'gpt-5.6-luna' },
    { ...codexDefault, provider: 'claude-code', model: 'claude-sonnet-5-5' },
]

export function UtilityModelSettingsEditor({ projectPath }: { projectPath: string | null }) {
    const editor = useModelSettingsEditor(undefined, 'utility_models')
    const profiles = useLlmProfiles()
    const discovery = useModelOptions(projectPath)
    const utilityDefault = UTILITY_DEFAULTS.find(({ provider }) => discovery?.payload?.providers?.[provider]?.status === 'available') ?? codexDefault
    const invalidModel = !!editor.draft && !isModelSelectionValid(editor.draft.llm_profile || editor.draft.provider || '', editor.draft.model, profiles)
    return <Card className="gap-4 py-4">
        <CardHeader className="gap-1 px-4"><h3 className="text-lg font-light">Utility model</h3></CardHeader>
        <CardContent className="space-y-3 px-4 pt-0">
            <p className="text-xs text-muted-foreground">A small, fast model Spark uses for housekeeping, such as naming threads. Off until you choose one; Spark does not fall back to the chat model.</p>
            {editor.pending ? <p role="status">Saving or reloading settings…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
            <fieldset disabled={!editor.saved || editor.pending} className="space-y-3">
                <Label className="flex items-center gap-2 text-sm"><Switch checked={editor.draft !== null}
                    onCheckedChange={(checked) => editor.setDraft(checked ? { ...utilityDefault } : null)} />Use a utility model</Label>
                {editor.draft && <ModelChooser inherited={codexDefault} disabled={!editor.saved || editor.pending} value={editor.draft} onChange={next => editor.setDraft(next.provider || next.llm_profile ? next : { ...next, provider: 'codex' })} projectPath={projectPath} inheritLabel="Provider default" invalidModel={!!invalidModel} />}
            </fieldset>
            <div className="flex flex-wrap gap-2">
                <Button aria-label="Save utility model" size="sm" disabled={!editor.dirty || editor.pending || !!invalidModel} onClick={() => void editor.save()}>Save</Button>
                <Button aria-label="Discard utility model changes" size="sm" variant="outline" disabled={!editor.saved || editor.pending} onClick={() => void editor.discard()}>Discard</Button>
            </div>
            {!editor.draft && editor.saved?.repair_defaults && <Button variant="outline" disabled={editor.pending} onClick={() => editor.setDraft(editor.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {editor.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
            <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
        </CardContent>
    </Card>
}
