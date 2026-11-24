import { runPresentationChoices, type RunPresentation } from './services/clientPreferences'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { useClientPreferencesEditor } from './hooks/useClientPreferencesEditor'
import type { Appearance } from '@/lib/theme'
import type { EditorMode } from '@/state/store-types'
import { SaveStatus } from './SaveStatus'

// The page's heading names these preferences, so each group is a section heading of its own,
// sized and spaced like the other categories' section headings, above the field labels.
const groupHeading = 'pt-6 text-base font-semibold first:pt-0'

export function ClientPreferencesEditor() {
    const editor = useClientPreferencesEditor()
    const hasCustomLayout = editor.draft?.editor_sidebar_width != null
        || editor.draft?.run_presentation?.graph_height != null
        || editor.draft?.home_sidebar_primary_split_ratio != null
    return <Card>
        <CardContent className="space-y-3">
            {editor.pending ? <p role="status">Updating…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
            <fieldset disabled={!editor.draft || editor.pending} className="space-y-3">
                <h2 className={groupHeading}>Appearance</h2>
                <Field><FieldLabel htmlFor="preference-appearance">Theme</FieldLabel>
                    <NativeSelect id="preference-appearance" value={editor.theme}
                        onChange={(event) => editor.setTheme(event.target.value as Appearance)}>
                        <option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option>
                    </NativeSelect>
                </Field>
                <h2 className={groupHeading}>Editor</h2>
                <p className="-mt-1.5 text-sm text-muted-foreground">Defaults for newly opened flows.</p>
                <Field><FieldLabel htmlFor="preference-editor-mode">Editor mode</FieldLabel>
                    <NativeSelect id="preference-editor-mode" value={editor.draft?.editor_mode ?? 'structured'}
                        onChange={(event) => { const mode = event.target.value as EditorMode; editor.setDraft((draft) => draft && ({ ...draft, editor_mode: mode })) }}>
                        <option value="structured">Structured</option><option value="files">Files</option>
                    </NativeSelect>
                </Field>
                {([
                    ['show_advanced_controls', 'Show advanced controls'],
                    ['expand_child_flows', 'Expand child flows'],
                    ['graph_settings_open', 'Open graph settings panel'],
                ] as const).map(([key, label]) => <Label key={key} className="flex items-center gap-2 text-sm">
                    <Switch id={`preference-${key}`} checked={editor.draft?.[key] ?? false}
                        onCheckedChange={(checked) => editor.setDraft((draft) => draft && ({ ...draft, [key]: checked }))} />
                    {label}
                </Label>)}
                <h2 className={groupHeading}>Transcripts</h2>
                <Label className="flex items-center gap-2 text-sm">
                    <Switch id="preference-show_thinking" checked={editor.draft?.show_thinking ?? true}
                        onCheckedChange={(checked) => editor.setDraft((draft) => draft && ({ ...draft, show_thinking: checked }))} />
                    Show thinking
                </Label>
                <h2 className={groupHeading}>Runs</h2>
                {(Object.keys(runPresentationChoices) as (keyof typeof runPresentationChoices)[]).map((key) => <Field key={key}>
                    <FieldLabel htmlFor={`run-preference-${key}`}>{{ sort: 'Run sort order' }[key]}</FieldLabel>
                    <NativeSelect id={`run-preference-${key}`} value={editor.draft?.run_presentation?.[key] ?? 'newest'} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, run_presentation: { ...draft.run_presentation, [key]: event.target.value } as RunPresentation }))}>
                        {runPresentationChoices[key].map((value) => <option key={value} value={value}>{value[0].toUpperCase() + value.slice(1)}</option>)}
                    </NativeSelect>
                </Field>)}
                <h2 className={groupHeading}>Layout</h2>
                <p className="-mt-1.5 text-sm text-muted-foreground">Panel widths and graph heights are saved automatically when you drag their edges.</p>
                <Button size="sm" variant="outline" disabled={!hasCustomLayout}
                    onClick={() => editor.setDraft((draft) => draft && ({
                        ...draft,
                        editor_sidebar_width: null,
                        ...(draft.home_sidebar_primary_split_ratio !== undefined ? { home_sidebar_primary_split_ratio: null } : {}),
                        ...(draft.run_presentation ? { run_presentation: { ...draft.run_presentation, graph_height: null } } : {}),
                    }))}>Reset layout</Button>
            </fieldset>
            <div className="flex flex-wrap gap-2">
                <Button size="sm" aria-label="Save client preferences" disabled={!editor.dirty || editor.pending || editor.invalidWidth || editor.invalidGraphHeight} onClick={() => void editor.save()}>Save</Button>
                <Button size="sm" aria-label="Discard client preferences changes" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard</Button>
            </div>
            <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
        </CardContent>
    </Card>
}
