import { useState } from 'react'

import { InlineError } from '@/components/app/inline-error'
import { SidePanelRow } from '@/components/app/side-panel-row'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { FlowFile } from '@/lib/attractorClient'

export interface FlowFilesError {
    files: string[]
    message: string
}

/** The files whose draft differs from their saved text. */
export function editedFlowFiles(files: FlowFile[], drafts: Record<string, string>): FlowFile[] {
    // A draft matching its file (as after a save and reload) is not an edit.
    return files
        .filter((file) => drafts[file.name] !== undefined && drafts[file.name] !== file.content)
        .map((file) => ({ name: file.name, content: drafts[file.name] }))
}

/**
 * The flow's files beside an editor holding the selected file's exact text. The drafts and
 * the save error belong to the editor, so they outlive this view across mode switches.
 */
export function FlowFilesView({ files, drafts, error, selectedNodeId, onDraftChange, onSave }: {
    files: FlowFile[]
    drafts: Record<string, string>
    error: FlowFilesError | null
    /** A node selected on the canvas selects its `<node>.md`. */
    selectedNodeId: string | null
    onDraftChange: (name: string, content: string) => void
    /** Saves the edited files. */
    onSave: () => Promise<unknown>
}) {
    const nodeFile = (nodeId: string | null) => (nodeId ? `${nodeId}.md` : null)
    const [selectedName, setSelectedName] = useState(() => nodeFile(selectedNodeId))
    const [lastNodeId, setLastNodeId] = useState(selectedNodeId)
    const [saving, setSaving] = useState(false)

    if (selectedNodeId !== lastNodeId) {
        setLastNodeId(selectedNodeId)
        if (selectedNodeId) setSelectedName(nodeFile(selectedNodeId))
    }

    const selected = files.find((file) => file.name === selectedName) ?? files[0] ?? null
    const edited = editedFlowFiles(files, drafts)
    const isEdited = (name: string) => edited.some((file) => file.name === name)

    const save = async () => {
        setSaving(true)
        await onSave()
        setSaving(false)
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
                        onChange={(event) => onDraftChange(selected.name, event.target.value)}
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
