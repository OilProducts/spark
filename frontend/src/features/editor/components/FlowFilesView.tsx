import { useState } from 'react'
import { FilePlus, Trash2, Undo2 } from 'lucide-react'

import { useDialogController } from '@/components/app/dialog-controller'
import { InlineError } from '@/components/app/inline-error'
import { SidePanelRow } from '@/components/app/side-panel-row'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import type { FlowFile } from '@/lib/attractorClient'
import { cn } from '@/lib/utils'
import { flowProblems } from '../model/flowLoadFailure'

export interface FlowFilesError {
    files: string[]
    message: string
}

/** Problems as a list, or as one line when there is only one. */
export function ProblemList({ problems }: { problems: string[] }) {
    return problems.length === 1
        ? <p className="[overflow-wrap:anywhere]">{problems[0]}</p>
        : <ul className="list-disc space-y-0.5 pl-4 [overflow-wrap:anywhere]">{problems.map((problem) => <li key={problem}>{problem}</li>)}</ul>
}

/** Drafts by file name: a file's new text, or `null` for a node file to delete. */
export type FlowFileDrafts = Record<string, string | null>

export interface FlowFileChanges {
    /** Edited and new files, with their text. */
    files: FlowFile[]
    /** Node files to delete. */
    deleted: string[]
}

/** What saving the drafts changes; a draft matching its file (as after a save and reload) is no change. */
export function flowFileChanges(files: FlowFile[], drafts: FlowFileDrafts): FlowFileChanges {
    const saved = new Map(files.map((file) => [file.name, file.content]))
    const changes: FlowFileChanges = { files: [], deleted: [] }
    for (const [name, draft] of Object.entries(drafts)) {
        if (draft === null) {
            if (saved.has(name)) changes.deleted.push(name)
        } else if (saved.get(name) !== draft) {
            changes.files.push({ name, content: draft })
        }
    }
    return changes
}

export const hasFlowFileChanges = (changes: FlowFileChanges) => changes.files.length + changes.deleted.length > 0

// A new node file's id: a letter or underscore first, then letters, digits, `_` or `-`, as the
// server requires of node ids.
const nodeIdFrom = (text: string) => text.trim().toLowerCase().replace(/\.md$/i, '').replace(/[^A-Za-z0-9_-]+/g, '_').replace(/^(?=[^A-Za-z_])/, '_')

const newNodeFile = (nodeId: string) => `---
type: agent_task
title: ${nodeId.replace(/[_-]+/g, ' ').trim().replace(/^./, (first) => first.toUpperCase()) || nodeId}
contracts:
  response: decision_envelope
---

Describe what this step should do.
`

/**
 * The flow's files beside an editor holding the selected file's exact text. The drafts and
 * the save error belong to the editor, so they outlive this view across mode switches. Node
 * files can be added and deleted; `flow.md` always stays.
 */
export function FlowFilesView({ files, drafts, error, loadProblems, selectedNodeId, onDraftChange, onSave, className }: {
    files: FlowFile[]
    drafts: FlowFileDrafts
    error: FlowFilesError | null
    /** Why the flow won't open, while it doesn't: fixing these files and saving opens it. */
    loadProblems?: string[] | null
    /** A node selected on the canvas selects its `<node>.md`. */
    selectedNodeId: string | null
    /** A file's new text, or `null` to delete it (or drop a new file not saved yet). */
    onDraftChange: (name: string, content: string | null) => void
    /** Saves the edited files. */
    onSave: () => Promise<unknown>
    className?: string
}) {
    const { prompt } = useDialogController()
    const nodeFile = (nodeId: string | null) => (nodeId ? `${nodeId}.md` : null)
    const [selectedName, setSelectedName] = useState(() => nodeFile(selectedNodeId))
    const [lastNodeId, setLastNodeId] = useState(selectedNodeId)
    const [saving, setSaving] = useState(false)

    if (selectedNodeId !== lastNodeId) {
        setLastNodeId(selectedNodeId)
        if (selectedNodeId) setSelectedName(nodeFile(selectedNodeId))
    }

    const savedNames = new Set(files.map((file) => file.name))
    const newNames = Object.keys(drafts).filter((name) => !savedNames.has(name) && drafts[name] !== null).sort()
    const names = [...files.map((file) => file.name), ...newNames]
    const selectedFile = names.includes(selectedName ?? '') ? selectedName : names[0] ?? null
    const savedText = (name: string) => files.find((file) => file.name === name)?.content ?? ''
    const changes = flowFileChanges(files, drafts)
    const status = (name: string) => (
        changes.deleted.includes(name) ? 'deleted'
            : !savedNames.has(name) ? 'new'
                : changes.files.some((file) => file.name === name) ? 'edited' : undefined
    )
    const selectedDeleted = selectedFile !== null && drafts[selectedFile] === null

    const addFile = async () => {
        const answer = await prompt({
            title: 'New node file',
            description: 'Each node of the flow is a file named after its ID. Save to add it.',
            label: 'Node ID',
            placeholder: 'review',
            confirmLabel: 'Add file',
            requireInput: true,
        })
        if (!answer?.trim()) return
        const id = nodeIdFrom(answer)
        // A taken name gets a number, so a new file never replaces one.
        const taken = (name: string) => names.some((existing) => existing.toLowerCase() === name.toLowerCase())
        let name = `${id}.md`
        for (let n = 2; taken(name); n += 1) name = `${id}_${n}.md`
        onDraftChange(name, newNodeFile(name.slice(0, -3)))
        setSelectedName(name)
    }
    const save = async () => {
        setSaving(true)
        await onSave()
        setSaving(false)
    }

    return (
        // Below md the canvas is hidden and the list stacks above a full-width editor.
        <section data-testid="flow-files-view" aria-label="Flow files" className={cn('flex min-h-0 w-full flex-1 flex-col md:w-[min(56rem,66%)] md:flex-none md:flex-row md:border-l', className)}>
            <ul data-testid="flow-file-list" aria-label="Files" className="flex max-h-24 shrink-0 flex-col gap-0.5 overflow-y-auto border-b p-2 md:max-h-none md:w-40 md:border-b-0 md:border-r">
                {names.map((name) => (
                    <li key={name}>
                        <SidePanelRow
                            data-testid="flow-file"
                            data-file={name}
                            title={<span className={status(name) === 'deleted' ? 'text-muted-foreground line-through' : undefined}>{name}</span>}
                            meta={status(name)}
                            selected={name === selectedFile}
                            onClick={() => setSelectedName(name)}
                        />
                    </li>
                ))}
                <li className="pt-1">
                    <Button type="button" variant="ghost" size="sm" data-testid="flow-file-add" className="w-full justify-start text-muted-foreground" onClick={() => void addFile()}>
                        <FilePlus aria-hidden="true" />
                        New file
                    </Button>
                </li>
            </ul>
            <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2 p-3">
                {/* A refused save lists what is left to fix under the editor instead, so the two never disagree. */}
                {loadProblems && !error ? (
                    <InlineError data-testid="flow-files-load-problems" title="This flow couldn’t be opened">
                        <p>Fix {loadProblems.length === 0 ? 'its files' : loadProblems.length === 1 ? 'this problem in its files' : `these ${loadProblems.length} problems in its files`}, then save to open it.</p>
                        {loadProblems.length > 0 ? <div className="mt-1"><ProblemList problems={loadProblems} /></div> : null}
                    </InlineError>
                ) : null}
                {selectedFile && selectedDeleted ? (
                    <div data-testid="flow-file-deleted" className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 rounded-md border border-dashed text-sm text-muted-foreground">
                        <p>{selectedFile} is deleted when you save.</p>
                        <Button type="button" size="sm" variant="outline" onClick={() => onDraftChange(selectedFile, savedText(selectedFile))}>
                            <Undo2 aria-hidden="true" />
                            Restore
                        </Button>
                    </div>
                ) : selectedFile ? (
                    <Textarea
                        data-testid="flow-file-editor"
                        aria-label={selectedFile}
                        value={drafts[selectedFile] ?? savedText(selectedFile)}
                        onChange={(event) => onDraftChange(selectedFile, event.target.value)}
                        className="min-h-0 flex-1 resize-none font-mono text-sm leading-5"
                        spellCheck={false}
                    />
                ) : <p className="text-sm text-muted-foreground">This flow has no files.</p>}
                {error ? (
                    <InlineError dense data-testid="flow-files-error" title={`Not saved: ${error.files.join(', ')}`}>
                        <ProblemList problems={flowProblems(error.message)} />
                    </InlineError>
                ) : null}
                <div className="flex items-center justify-end gap-2">
                    {/* flow.md is the flow itself; a new file not saved yet is just dropped. */}
                    {selectedFile && selectedFile !== 'flow.md' && !selectedDeleted ? (
                        <Button
                            data-testid="flow-file-delete"
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="mr-auto text-muted-foreground"
                            onClick={() => onDraftChange(selectedFile, null)}
                        >
                            <Trash2 aria-hidden="true" />
                            {savedNames.has(selectedFile) ? 'Delete file' : 'Discard new file'}
                        </Button>
                    ) : null}
                    <Button
                        data-testid="flow-files-save"
                        type="button"
                        size="sm"
                        disabled={saving || !hasFlowFileChanges(changes)}
                        onClick={() => void save()}
                    >
                        {saving ? 'Saving…' : 'Save'}
                    </Button>
                </div>
            </div>
        </section>
    )
}
