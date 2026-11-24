import { useEffect, useState } from 'react'
import { ChevronRight, Folder, FolderOpen, House, MoveUpRight } from 'lucide-react'

import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from '@/components/ui/dialog'
import { InlineError } from '@/components/app/inline-error'
import { Empty, EmptyDescription, EmptyHeader } from '@/components/ui/empty'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { ProjectBrowseEntryResponse } from '@/lib/workspaceClient'

type ProjectBrowserDialogProps = {
    open: boolean
    currentPath: string | null
    parentPath: string | null
    roots: string[]
    entries: ProjectBrowseEntryResponse[]
    errorMessage: string | null
    isLoading: boolean
    onBrowse: (path?: string) => void
    onOpenChange: (open: boolean) => void
    onSelectCurrentFolder: () => void | Promise<void>
}

type BreadcrumbSegment = {
    label: string
    path: string
}

function buildBreadcrumbSegments(path: string): BreadcrumbSegment[] {
    // A Windows path roots at its drive (C:\) or UNC share (\\server\share), and its crumbs keep the path's own separator.
    const windows = path.match(/^(?:([A-Za-z]:)|[/\\]{2}([^/\\]+)[/\\]+([^/\\]+))/)
    const separator = windows && path.includes('\\') ? '\\' : '/'
    const root = !windows ? '/'
        : windows[1] ? `${windows[1]}${separator}`
            : `${separator}${separator}${windows[2]}${separator}${windows[3]}`
    const segments = path.slice(windows?.[0].length ?? 0).split(windows ? /[/\\]/ : '/').filter(Boolean)
    const base = root.endsWith(separator) ? root : `${root}${separator}`
    return [
        { label: root, path: root },
        ...segments.map((segment, index) => ({
            label: segment,
            path: `${base}${segments.slice(0, index + 1).join(separator)}`,
        })),
    ]
}

export function ProjectBrowserDialog({
    open,
    currentPath,
    parentPath,
    roots,
    entries,
    errorMessage,
    isLoading,
    onBrowse,
    onOpenChange,
    onSelectCurrentFolder,
}: ProjectBrowserDialogProps) {
    const breadcrumbSegments = currentPath ? buildBreadcrumbSegments(currentPath) : []
    // The path box shows where the browser is; type or paste a path there to go straight to it.
    const [pathDraft, setPathDraft] = useState(currentPath ?? '')
    useEffect(() => { setPathDraft(currentPath ?? '') }, [currentPath])
    const typedPath = pathDraft.trim()
    const typedElsewhere = typedPath.length > 0 && typedPath !== currentPath
    const canSelectCurrentFolder = Boolean(currentPath) && !isLoading && !typedElsewhere

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            {open ? (
                <DialogContent
                    data-testid="project-browser-dialog"
                    // One shrinkable column, so a long path wraps inside the dialog instead of widening it past its buttons.
                    className="grid-cols-[minmax(0,1fr)] gap-0 overflow-hidden p-0 sm:max-w-3xl"
                >
                    <DialogHeader className="border-b border-border/70 px-6 py-5">
                        <DialogTitle>Add a project</DialogTitle>
                        <DialogDescription>
                            Type or paste the project’s folder path, or browse to it, then select it.
                        </DialogDescription>
                    </DialogHeader>

                    <div className="flex flex-col gap-4 px-6 py-5">
                        <div className="flex flex-wrap items-start justify-between gap-3">
                            <div className="min-w-0 flex-1 space-y-2">
                                <div className="flex flex-wrap items-center gap-1.5" data-testid="project-browser-breadcrumbs">
                                    {breadcrumbSegments.length === 0 ? (
                                        <span className="text-sm text-muted-foreground">Loading current directory…</span>
                                    ) : (
                                        breadcrumbSegments.map((segment, index) => (
                                            <div key={segment.path} className="flex min-w-0 max-w-full items-center gap-1.5">
                                                {index === 0 && segment.label === '/' ? (
                                                    <Button
                                                        type="button"
                                                        variant="ghost"
                                                        size="xs"
                                                        onClick={() => onBrowse(segment.path)}
                                                        disabled={isLoading}
                                                    >
                                                        <House className="h-3 w-3" />
                                                        Root
                                                    </Button>
                                                ) : (
                                                    <Button
                                                        type="button"
                                                        variant={index === breadcrumbSegments.length - 1 ? 'secondary' : 'ghost'}
                                                        size="xs"
                                                        onClick={() => onBrowse(segment.path)}
                                                        disabled={isLoading}
                                                        className="min-w-0 max-w-full"
                                                    >
                                                        <span className="truncate">{segment.label}</span>
                                                    </Button>
                                                )}
                                                {index < breadcrumbSegments.length - 1 ? (
                                                    <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
                                                ) : null}
                                            </div>
                                        ))
                                    )}
                                </div>
                                <form
                                    className="flex gap-2"
                                    onSubmit={(event) => {
                                        event.preventDefault()
                                        if (typedElsewhere && !isLoading) onBrowse(typedPath)
                                    }}
                                >
                                    <Input
                                        data-testid="project-browser-current-path"
                                        aria-label="Folder path"
                                        value={pathDraft}
                                        onChange={(event) => setPathDraft(event.target.value)}
                                        placeholder={currentPath ? 'Type or paste a folder path' : 'Resolving directory…'}
                                        spellCheck={false}
                                        autoComplete="off"
                                        autoCapitalize="off"
                                        className="h-8 min-w-0 flex-1 font-mono text-xs"
                                    />
                                    <Button type="submit" variant="outline" size="sm" data-testid="project-browser-go-button" disabled={!typedElsewhere || isLoading}>
                                        Go
                                    </Button>
                                </form>
                            </div>

                            <Button
                                data-testid="project-browser-parent-button"
                                type="button"
                                variant="outline"
                                size="xs"
                                onClick={() => {
                                    if (parentPath) {
                                        onBrowse(parentPath)
                                    }
                                }}
                                disabled={!parentPath || isLoading}
                            >
                                <MoveUpRight className="h-3.5 w-3.5" />
                                Parent
                            </Button>
                        </div>

                        {roots.length > 0 ? (
                            <div className="flex flex-wrap items-center gap-2" data-testid="project-browser-root-shortcuts">
                                {roots.map((root) => (
                                    <Button
                                        key={root}
                                        type="button"
                                        variant={root === currentPath ? 'secondary' : 'outline'}
                                        size="xs"
                                        onClick={() => onBrowse(root)}
                                        disabled={isLoading}
                                        data-testid={`project-browser-root-${root.replace(/[^A-Za-z0-9_-]+/g, '-')}`}
                                    >
                                        <FolderOpen className="h-3.5 w-3.5" />
                                        <span className="max-w-56 truncate font-mono text-xs">{root}</span>
                                    </Button>
                                ))}
                            </div>
                        ) : null}

                        {errorMessage ? (
                            <InlineError data-testid="project-browser-error">{errorMessage}</InlineError>
                        ) : null}

                        <div
                            data-testid="project-browser-entry-list"
                            className="max-h-[22rem] overflow-y-auto rounded-lg border border-border/70 p-2"
                        >
                            {isLoading ? (
                                <div
                                    data-testid="project-browser-loading"
                                    className="rounded-md border border-dashed border-border/70 px-4 py-8 text-center text-sm text-muted-foreground"
                                >
                                    Loading directories…
                                </div>
                            ) : entries.length === 0 ? (
                                <Empty className="text-sm text-muted-foreground">
                                    <EmptyHeader>
                                        <EmptyDescription>No subdirectories in this location.</EmptyDescription>
                                    </EmptyHeader>
                                </Empty>
                            ) : (
                                <ul className="space-y-1">
                                    {entries.map((entry) => (
                                        <li key={entry.path}>
                                            <Button
                                                data-testid={`project-browser-entry-${entry.name}`}
                                                type="button"
                                                variant="ghost"
                                                size="sm"
                                                onClick={() => onBrowse(entry.path)}
                                                className="h-auto w-full justify-between rounded-md px-3 py-2 text-left"
                                            >
                                                <span className="flex min-w-0 items-center gap-2">
                                                    <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                                                    <span className="truncate">{entry.name}</span>
                                                </span>
                                                <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
                                            </Button>
                                        </li>
                                    ))}
                                </ul>
                            )}
                        </div>
                    </div>

                    <DialogFooter className="border-t border-border/70 px-6 py-4">
                        <Button
                            data-testid="project-browser-select-button"
                            type="button"
                            onClick={() => {
                                void onSelectCurrentFolder()
                            }}
                            disabled={!canSelectCurrentFolder}
                            title={typedElsewhere ? 'Go to the typed folder first' : undefined}
                        >
                            <FolderOpen className="h-4 w-4" />
                            Select this folder
                        </Button>
                    </DialogFooter>
                </DialogContent>
            ) : null}
        </Dialog>
    )
}
