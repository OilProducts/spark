import { useState } from 'react'

import { InlineError } from '@/components/app/inline-error'
import { SidePanelRow } from '@/components/app/side-panel-row'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { FlowFile } from '@/lib/attractorClient'
import { useStore } from '@/store'

/** The flow's files beside an editor holding the selected file's exact text. */
export function FlowFilesView({ files, selectedNodeId, onSave }: {
    files: FlowFile[]
    /** A node selected on the canvas selects its `<node>.md`. */
    selectedNodeId: string | null
    /** Saves the edited files; resolves false when the save was refused. */
    onSave: (files: FlowFile[]) => Promise<boolean>
}) {
    const nodeFile = (nodeId: string | null) => (nodeId ? `${nodeId}.md` : null)
    const [selectedName, setSelectedName] = useState(() => nodeFile(selectedNodeId))
    const [lastNodeId, setLastNodeId] = useState(selectedNodeId)
    const [drafts, setDrafts] = useState<Record<string, string>>({})
    const [error, setError] = useState<{ files: string[]; message: string } | null>(null)
    const [saving, setSaving] = useState(false)

    if (selectedNodeId !== lastNodeId) {
        setLastNodeId(selectedNodeId)
        if (selectedNodeId) setSelectedName(nodeFile(selectedNodeId))
    }

    const selected = files.find((file) => file.name === selectedName) ?? files[0] ?? null
    // A draft matching its file (as after a save and reload) is not an edit.
    const edited = files.filter((file) => drafts[file.name] !== undefined && drafts[file.name] !== file.content)
    const isEdited = (name: string) => edited.some((file) => file.name === name)

    const save = async () => {
        const changed = edited.map((file) => ({ name: file.name, content: drafts[file.name] }))
        setSaving(true)
        const saved = await onSave(changed)
        setSaving(false)
        setError(saved ? null : {
            files: changed.map((file) => file.name),
            message: useStore.getState().saveErrorMessage || 'The files were not saved.',
        })
    }

    return (
        <section data-testid="flow-files-view" aria-label="Flow files" className="flex min-h-0 w-[min(44rem,60%)] shrink-0 border-l">
            <ul data-testid="flow-file-list" className="flex w-44 shrink-0 flex-col gap-0.5 overflow-y-auto border-r p-2">
                {files.map((file) => (
                    <li key={file.name}>
                        <SidePanelRow
                            data-testid="flow-file"
                            data-file={file.name}
                            title={file.name}
                            meta={isEdited(file.name) ? 'edited' : undefined}
                            selected={file.name === selected?.name}
                            onClick={() => setSelectedName(file.name)}
                        />
                    </li>
                ))}
            </ul>
            <div className="flex min-w-0 flex-1 flex-col gap-2 p-3">
                {selected ? (
                    <Textarea
                        data-testid="flow-file-editor"
                        aria-label={selected.name}
                        value={drafts[selected.name] ?? selected.content}
                        onChange={(event) => {
                            const content = event.target.value
                            setDrafts((current) => ({ ...current, [selected.name]: content }))
                        }}
                        className="min-h-0 flex-1 resize-none font-mono text-sm leading-5"
                        spellCheck={false}
                    />
                ) : <p className="text-sm text-muted-foreground">This flow has no files.</p>}
                {error ? (
                    <InlineError dense data-testid="flow-files-error" title={error.files.join(', ')}>{error.message}</InlineError>
                ) : null}
                <div className="flex items-center justify-end gap-2">
                    <Button
                        data-testid="flow-files-save"
                        type="button"
                        size="sm"
                        disabled={saving || edited.length === 0}
                        onClick={() => void save()}
                    >
                        {saving ? 'Saving…' : 'Save'}
                    </Button>
                </div>
            </div>
        </section>
    )
}
