import { runPresentationChoices, type RunPresentation } from './services/clientPreferences'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Field, FieldError, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { useClientPreferencesEditor } from './hooks/useClientPreferencesEditor'
import type { Appearance } from '@/lib/theme'
import type { EditorMode } from '@/state/store-types'
import { SaveStatus } from './SaveStatus'

// The page's heading names these preferences, so each group is a section heading of its own,
// sized and spaced like the other categories' section headings, above the field labels.
const groupHeading = 'pt-6 text-base font-semibold first:pt-0'

export function ClientPreferencesEditor() {
    const editor = useClientPreferencesEditor()
    return <Card>
        <CardContent className="space-y-3">
            {editor.pending ? <p role="status">Updating…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
            <fieldset disabled={!editor.draft || editor.pending} className="space-y-3">
                <h2 className={groupHeading}>Appearance</h2>
                <Field><FieldLabel htmlFor="preference-appearance">Theme</FieldLabel>
                    <NativeSelect id="preference-appearance" value={editor.draft?.appearance ?? 'system'}
                        onChange={(event) => { const appearance = event.target.value as Appearance; editor.setDraft((draft) => draft && ({ ...draft, appearance })) }}>
                        <option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option>
                    </NativeSelect>
                </Field>
                <h2 className={groupHeading}>Editor</h2>
                <p className="-mt-1.5 text-sm text-muted-foreground">Defaults for newly opened flows.</p>
                <Field><FieldLabel htmlFor="preference-editor-mode">Editor mode</FieldLabel>
                    <NativeSelect id="preference-editor-mode" value={editor.draft?.editor_mode ?? ''}
                        onChange={(event) => { const mode = event.target.value as EditorMode | ''; editor.setDraft((draft) => draft && ({ ...draft, editor_mode: mode || null })) }}>
                        <option value="">Default (structured)</option><option value="structured">Structured</option><option value="files">Files</option>
                    </NativeSelect>
                </Field>
                {([
                    ['show_advanced_controls', 'Show advanced controls'],
                    ['expand_child_flows', 'Expand child flows'],
                    ['graph_settings_open', 'Open graph settings panel'],
                ] as const).map(([key, label]) => <Field key={key}>
                    <FieldLabel htmlFor={`preference-${key}`}>{label}</FieldLabel>
                    <NativeSelect id={`preference-${key}`} value={editor.draft?.[key] == null ? '' : String(editor.draft[key])}
                        onChange={(event) => { const value = event.target.value; editor.setDraft((draft) => draft && ({ ...draft, [key]: value === '' ? null : value === 'true' })) }}>
                        <option value="">Default (off)</option><option value="true">On</option><option value="false">Off</option>
                    </NativeSelect>
                </Field>)}
                <h2 className={groupHeading}>Transcripts</h2>
                <Field><FieldLabel htmlFor="preference-show_thinking">Show thinking</FieldLabel>
                    <NativeSelect id="preference-show_thinking" value={editor.draft?.show_thinking == null ? '' : String(editor.draft.show_thinking)}
                        onChange={(event) => { const value = event.target.value; editor.setDraft((draft) => draft && ({ ...draft, show_thinking: value === '' ? null : value === 'true' })) }}>
                        <option value="">Default (on)</option><option value="true">On</option><option value="false">Off</option>
                    </NativeSelect>
                </Field>
                <h2 className={groupHeading}>Layout</h2>
                <p className="-mt-1.5 text-sm text-muted-foreground">You can also resize these by dragging their edges. Leave a size blank to use its default.</p>
                <Field><FieldLabel htmlFor="preference-sidebar-width">Flow editor panel width</FieldLabel>
                    <div className="flex items-center gap-2">
                        <Input id="preference-sidebar-width" className="w-40" type="number" min={256} max={560} step={1}
                            aria-invalid={editor.invalidWidth} aria-describedby="preference-sidebar-unit preference-sidebar-help preference-sidebar-error"
                            value={editor.draft?.editor_sidebar_width ?? ''}
                            onChange={(event) => { const width = event.target.value === '' ? null : Number(event.target.value); editor.setDraft((draft) => draft && ({ ...draft, editor_sidebar_width: width })) }} />
                        <span id="preference-sidebar-unit" className="text-sm text-muted-foreground">pixels</span>
                    </div>
                    <p id="preference-sidebar-help" className="text-xs text-muted-foreground">The properties panel beside the flow graph. 256 to 560; default 288.</p>
                    {editor.invalidWidth && <FieldError id="preference-sidebar-error" role="alert">Choose a whole number from 256 to 560.</FieldError>}
                </Field>
                <Field><FieldLabel htmlFor="run-preference-graph-height">Run graph height</FieldLabel>
                    <div className="flex items-center gap-2">
                        <Input id="run-preference-graph-height" className="w-40" type="number" min={280} max={960} step={1}
                            aria-invalid={editor.invalidGraphHeight} aria-describedby="preference-graph-unit preference-graph-help preference-graph-error"
                            value={editor.draft?.run_presentation?.graph_height ?? ''}
                            onChange={(event) => { const height = event.target.value === '' ? null : Number(event.target.value); editor.setDraft((draft) => draft && ({ ...draft, run_presentation: { ...draft.run_presentation, graph_height: height } })) }} />
                        <span id="preference-graph-unit" className="text-sm text-muted-foreground">pixels</span>
                    </div>
                    <p id="preference-graph-help" className="text-xs text-muted-foreground">The flow graph on a run’s page. 280 to 960; default 512.</p>
                    {editor.invalidGraphHeight && <FieldError id="preference-graph-error" role="alert">Choose a whole number from 280 to 960.</FieldError>}
                </Field>
                <Button size="sm" variant="outline" disabled={editor.draft?.editor_sidebar_width == null && editor.draft?.run_presentation?.graph_height == null}
                    onClick={() => editor.setDraft((draft) => draft && ({ ...draft, editor_sidebar_width: null, run_presentation: { ...draft.run_presentation, graph_height: null } }))}>Reset layout</Button>
                <h2 className={groupHeading}>Runs</h2>
                {(Object.keys(runPresentationChoices) as (keyof typeof runPresentationChoices)[]).map((key) => <Field key={key}>
                    <FieldLabel htmlFor={`run-preference-${key}`}>{{ sort: 'Run sort order' }[key]}</FieldLabel>
                    <NativeSelect id={`run-preference-${key}`} value={editor.draft?.run_presentation?.[key] ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, run_presentation: { ...draft.run_presentation, [key]: event.target.value || null } as RunPresentation }))}>
                        <option value="">Default</option>{runPresentationChoices[key].map((value) => <option key={value} value={value}>{value[0].toUpperCase() + value.slice(1)}</option>)}
                    </NativeSelect>
                </Field>)}
            </fieldset>
            <div className="flex flex-wrap gap-2">
                <Button size="sm" aria-label="Save client preferences" disabled={!editor.dirty || editor.pending || editor.invalidWidth || editor.invalidGraphHeight} onClick={() => void editor.save()}>Save</Button>
                <Button size="sm" variant="ghost" aria-label="Discard client preferences changes" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard</Button>
            </div>
            <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
        </CardContent>
    </Card>
}
