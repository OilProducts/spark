import { ChevronRight, Folder, FolderOpen, House, MoveUpRight } from 'lucide-react'

import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from '@/components/ui/dialog'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
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
    if (path === '/') {
        return [{ label: '/', path }]
    }

    const windowsMatch = path.match(/^([A-Za-z]:)(\/.*)?$/)
    if (windowsMatch) {
        const drive = `${windowsMatch[1]}/`
        const segments = (windowsMatch[2] || '').split('/').filter(Boolean)
        return [
            { label: drive, path: drive },
            ...segments.map((segment, index) => ({
                label: segment,
                path: `${drive}${segments.slice(0, index + 1).join('/')}`,
            })),
        ]
    }

    const segments = path.split('/').filter(Boolean)
    return [
        { label: '/', path: '/' },
        ...segments.map((segment, index) => ({
            label: segment,
            path: `/${segments.slice(0, index + 1).join('/')}`,
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
    const canSelectCurrentFolder = Boolean(currentPath) && !isLoading

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            {open ? (
                <DialogContent
                    data-testid="project-browser-dialog"
                    className="max-w-3xl gap-0 overflow-hidden p-0"
                >
                    <DialogHeader className="border-b border-border/70 px-6 py-5">
                        <DialogTitle>Browse Spark Projects</DialogTitle>
                        <DialogDescription>
                            Select a directory Spark can access, then confirm the current folder to register it.
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
                                            <div key={segment.path} className="flex items-center gap-1.5">
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
                                                    >
                                                        {segment.label}
                                                    </Button>
                                                )}
                                                {index < breadcrumbSegments.length - 1 ? (
                                                    <ChevronRight className="h-3.5 w-3.5 text-muted-foreground" />
                                                ) : null}
                                            </div>
                                        ))
                                    )}
                                </div>
                                <p
                                    data-testid="project-browser-current-path"
                                    className="truncate rounded-md border border-border/70 bg-muted/30 px-3 py-2 font-mono text-xs text-foreground"
                                >
                                    {currentPath || 'Resolving directory…'}
                                </p>
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
                            <Alert data-testid="project-browser-error" variant="destructive">
                                <AlertDescription>{errorMessage}</AlertDescription>
                            </Alert>
                        ) : null}

                        <div
                            data-testid="project-browser-entry-list"
                            className="max-h-[22rem] overflow-y-auto rounded-lg border border-border/70 bg-muted/10 p-2"
                        >
                            {isLoading ? (
                                <div
                                    data-testid="project-browser-loading"
                                    className="rounded-md border border-dashed border-border/70 px-4 py-8 text-center text-sm text-muted-foreground"
                                >
                                    Loading directories…
                                </div>
                            ) : entries.length === 0 ? (
                                <div className="rounded-md border border-dashed border-border/70 px-4 py-8 text-center text-sm text-muted-foreground">
                                    No subdirectories in this location.
                                </div>
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
                        >
                            <FolderOpen className="h-4 w-4" />
                            Select This Folder
                        </Button>
                    </DialogFooter>
                </DialogContent>
            ) : null}
        </Dialog>
    )
}
