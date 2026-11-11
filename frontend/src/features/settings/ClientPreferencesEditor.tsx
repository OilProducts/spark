import { runPresentationChoices, type RunPresentation } from './services/clientPreferences'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { useClientPreferencesEditor } from './hooks/useClientPreferencesEditor'
import type { Appearance } from '@/lib/theme'
import type { EditorMode } from '@/state/store-types'
import { SaveStatus } from './SaveStatus'

// A rule and space above each group heading set it apart from the field labels under it.
const groupHeading = 'mt-3 border-t border-border pt-4 text-sm font-semibold first:mt-0 first:border-t-0 first:pt-0'

export function ClientPreferencesEditor() {
    const editor = useClientPreferencesEditor()
    return <Card>
        <CardHeader><h3 className="text-base font-semibold">Client preferences</h3></CardHeader>
        <CardContent className="space-y-3">
            {editor.pending ? <p role="status">Updating…</p> : !editor.saved && !editor.error ? <p role="status">Loading settings…</p> : null}
            <fieldset disabled={!editor.draft || editor.pending} className="space-y-3">
                <h4 className={groupHeading}>Appearance</h4>
                <Field><FieldLabel htmlFor="preference-appearance">Theme</FieldLabel>
                    <NativeSelect id="preference-appearance" value={editor.draft?.appearance ?? 'system'}
                        onChange={(event) => { const appearance = event.target.value as Appearance; editor.setDraft((draft) => draft && ({ ...draft, appearance })) }}>
                        <option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option>
                    </NativeSelect>
                </Field>
                <h4 className={groupHeading}>Editor</h4>
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
                <h4 className={groupHeading}>Transcripts</h4>
                <Field><FieldLabel htmlFor="preference-show_thinking">Show thinking</FieldLabel>
                    <NativeSelect id="preference-show_thinking" value={editor.draft?.show_thinking == null ? '' : String(editor.draft.show_thinking)}
                        onChange={(event) => { const value = event.target.value; editor.setDraft((draft) => draft && ({ ...draft, show_thinking: value === '' ? null : value === 'true' })) }}>
                        <option value="">Default (on)</option><option value="true">On</option><option value="false">Off</option>
                    </NativeSelect>
                </Field>
                <h4 className={groupHeading}>Layout</h4>
                <Field><FieldLabel htmlFor="preference-sidebar-width">Editor sidebar width (pixels)</FieldLabel>
                    <Input id="preference-sidebar-width" className="w-40" type="number" min={256} max={560} step={1}
                        aria-invalid={editor.invalidWidth} aria-describedby="preference-sidebar-help preference-sidebar-error"
                        value={editor.draft?.editor_sidebar_width ?? ''}
                        onChange={(event) => { const width = event.target.value === '' ? null : Number(event.target.value); editor.setDraft((draft) => draft && ({ ...draft, editor_sidebar_width: width })) }} />
                    <p id="preference-sidebar-help" className="text-xs text-muted-foreground">256–560 pixels. Leave blank for the default of 288. Applies after saving.</p>
                    {editor.invalidWidth && <p id="preference-sidebar-error" role="alert">Choose a whole number from 256 to 560.</p>}
                </Field>
                <Field><FieldLabel htmlFor="preference-home-split">Home sidebar primary split (0–1)</FieldLabel>
                    <Input id="preference-home-split" className="w-40" type="number" min={0} max={1} step={0.01}
                        aria-invalid={editor.invalidSplit} aria-describedby="preference-home-split-help preference-split-error"
                        value={editor.draft?.home_sidebar_primary_split_ratio ?? ''}
                        onChange={(event) => { const ratio = event.target.value === '' ? null : Number(event.target.value); editor.setDraft((draft) => draft && ({ ...draft, home_sidebar_primary_split_ratio: ratio })) }} />
                    <p id="preference-home-split-help" className="text-xs text-muted-foreground">Share of sidebar height for the top pane. Leave blank for automatic sizing.</p>
                    {editor.invalidSplit && <p id="preference-split-error" role="alert">Choose a number from 0 to 1.</p>}
                </Field>
                <h4 className={groupHeading}>Runs</h4>
                {(Object.keys(runPresentationChoices) as (keyof typeof runPresentationChoices)[]).map((key) => <Field key={key}>
                    <FieldLabel htmlFor={`run-preference-${key}`}>{{ sort: 'Run sort order' }[key]}</FieldLabel>
                    <NativeSelect id={`run-preference-${key}`} value={editor.draft?.run_presentation?.[key] ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, run_presentation: { ...draft.run_presentation, [key]: event.target.value || null } as RunPresentation }))}>
                        <option value="">Default</option>{runPresentationChoices[key].map((value) => <option key={value} value={value}>{value[0].toUpperCase() + value.slice(1)}</option>)}
                    </NativeSelect>
                </Field>)}
                <Field><FieldLabel htmlFor="run-preference-graph-height">Run graph height (pixels)</FieldLabel>
                    <Input id="run-preference-graph-height" className="w-40" aria-invalid={editor.invalidGraphHeight} aria-describedby={editor.invalidGraphHeight ? "preference-graph-error" : undefined} type="number" min={280} max={960} value={editor.draft?.run_presentation?.graph_height ?? ''} onChange={(event) => editor.setDraft((draft) => draft && ({ ...draft, run_presentation: { ...draft.run_presentation, graph_height: event.target.value === '' ? null : Number(event.target.value) } }))} />
                    {editor.invalidGraphHeight && <p id="preference-graph-error" role="alert">Choose a whole number from 280 to 960.</p>}
                </Field>
            </fieldset>
            <div className="flex flex-wrap gap-2">
                <Button size="sm" aria-label="Save client preferences" disabled={!editor.dirty || editor.pending || editor.invalidWidth || editor.invalidSplit || editor.invalidGraphHeight} onClick={() => void editor.save()}>Save</Button>
                <Button size="sm" variant="ghost" aria-label="Discard client preferences changes" disabled={editor.pending || (!editor.dirty && !editor.error)} onClick={() => void editor.discard()}>Discard</Button>
            </div>
            <SaveStatus message={editor.message} error={editor.error} dirty={editor.dirty} />
        </CardContent>
    </Card>
}
