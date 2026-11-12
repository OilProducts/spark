import { useEffect, useMemo, useState } from 'react'
import { ArrowLeft, Search } from 'lucide-react'

import { useStore } from '@/store'
import { openRun } from '@/app/useAttention'
import { InlineError } from '@/components/app/inline-error'
import { ListGroupLabel } from '@/components/app/side-panel-row'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { markProjectUsed } from '@/features/projects/hooks/usePersistProjectState'
import { defaultProjectChoice, orderProjects, projectLabel } from '@/features/projects/model/projectChoices'
import type { WorkspaceFlowResponse } from '@/lib/api/flowsApi'

import { useFlowSummaries } from '../hooks/useFlowSummaries'
import { LaunchPanel } from './LaunchPanel'

const RUN_FLOW_EVENT = 'spark:run-flow'

/** Opens the Run a flow dialog from anywhere, optionally in a project or on a flow. */
export function openRunFlowDialog(detail: { projectPath?: string | null; flowName?: string | null } = {}) {
    window.dispatchEvent(new CustomEvent(RUN_FLOW_EVENT, { detail }))
}

// "software-development/review-change" lists under "Software development".
const folderLabel = (name: string) => {
    const folder = name.includes('/') ? name.slice(0, name.indexOf('/')) : ''
    return folder ? folder.charAt(0).toUpperCase() + folder.slice(1).replace(/[-_]+/g, ' ') : 'Other flows'
}

/** Flows matching every word of `query` in their title, description or name, grouped by top folder. */
export function groupFlows(flows: WorkspaceFlowResponse[], query: string) {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean)
    const groups = new Map<string, WorkspaceFlowResponse[]>()
    for (const flow of flows) {
        const text = `${flow.title} ${flow.description} ${flow.name}`.toLowerCase()
        if (!words.every((word) => text.includes(word))) continue
        const label = folderLabel(flow.name)
        groups.set(label, [...(groups.get(label) ?? []), flow])
    }
    return [...groups.entries()].map(([label, items]) => ({
        label,
        items: items.sort((left, right) => (left.title || left.name).localeCompare(right.title || right.name)),
    }))
}

function FlowChooser({ projectPath, onProjectChange, onChoose }: {
    projectPath: string | null
    onProjectChange: (projectPath: string) => void
    onChoose: (flowName: string) => void
}) {
    const registry = useStore((state) => state.projectRegistry)
    const [query, setQuery] = useState('')
    const summaries = useFlowSummaries(projectPath)
    const groups = useMemo(() => groupFlows(summaries?.flows ?? [], query), [summaries, query])
    return (
        <div className="flex min-h-0 flex-1 flex-col gap-4">
            <div className="space-y-1">
                <DialogTitle>Run a flow</DialogTitle>
                <DialogDescription>Pick what to run. You can fill in its inputs on the next step.</DialogDescription>
            </div>
            <div className="grid gap-3 sm:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]">
                <div className="space-y-1.5 [&>[data-slot=native-select-wrapper]]:w-full">
                    <Label htmlFor="run-flow-project">Project</Label>
                    <NativeSelect
                        id="run-flow-project"
                        data-testid="run-flow-project-select"
                        value={projectPath ?? ''}
                        onChange={(event) => onProjectChange(event.target.value)}
                        size="sm"
                        className="w-full text-sm"
                    >
                        {orderProjects(registry).map((project) => (
                            <NativeSelectOption key={project.directoryPath} value={project.directoryPath}>
                                {projectLabel(registry, project.directoryPath)}
                            </NativeSelectOption>
                        ))}
                    </NativeSelect>
                </div>
                <div className="space-y-1.5">
                    <Label htmlFor="run-flow-search">Find a flow</Label>
                    <div className="relative">
                        <Search aria-hidden="true" className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                        <Input
                            id="run-flow-search"
                            data-testid="run-flow-search"
                            type="search"
                            value={query}
                            onChange={(event) => setQuery(event.target.value)}
                            placeholder="Search by name or what it does"
                            className="h-8 pl-8 text-sm"
                            autoFocus
                        />
                    </div>
                </div>
            </div>
            <div data-testid="run-flow-list" className="-mx-2 min-h-0 flex-1 overflow-y-auto px-2" aria-busy={summaries === null}>
                {summaries === null ? <p className="py-6 text-center text-sm text-muted-foreground">Loading flows…</p> : null}
                {summaries?.error ? <InlineError>{summaries.error}</InlineError> : null}
                {summaries && !summaries.error && groups.length === 0 ? (
                    <p className="py-6 text-center text-sm text-muted-foreground">
                        {query ? 'No flow matches that search.' : 'This project has no flows yet.'}
                    </p>
                ) : null}
                {groups.map((group) => (
                    <section key={group.label} aria-label={group.label} className="pb-3">
                        <ListGroupLabel><h3>{group.label}</h3></ListGroupLabel>
                        <ul className="space-y-0.5">
                            {group.items.map((flow) => (
                                <li key={flow.name}>
                                    <button
                                        type="button"
                                        data-testid="run-flow-option"
                                        data-flow-name={flow.name}
                                        onClick={() => onChoose(flow.name)}
                                        className="flex w-full flex-col items-start gap-0.5 rounded-md px-3 py-2 text-left outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
                                    >
                                        <span className="flex w-full min-w-0 flex-wrap items-baseline justify-between gap-x-3">
                                            <span className="min-w-0 text-sm font-medium">{flow.title || flow.name}</span>
                                            <span className="min-w-0 truncate font-mono text-xs text-muted-foreground">{flow.name}</span>
                                        </span>
                                        {flow.description ? <span className="line-clamp-2 text-xs text-muted-foreground">{flow.description}</span> : null}
                                    </button>
                                </li>
                            ))}
                        </ul>
                    </section>
                ))}
            </div>
        </div>
    )
}

/** Run a flow without opening it in the editor: choose a project and a flow, fill its inputs, run. */
export function RunFlowDialog() {
    const registry = useStore((state) => state.projectRegistry)
    const [request, setRequest] = useState<{ projectPath: string | null; flowName: string | null } | null>(null)

    useEffect(() => {
        const onOpen = (event: Event) => {
            const detail = (event as CustomEvent<{ projectPath?: string | null; flowName?: string | null }>).detail ?? {}
            const registry = useStore.getState().projectRegistry
            setRequest({ projectPath: defaultProjectChoice(registry, detail.projectPath), flowName: detail.flowName ?? null })
        }
        window.addEventListener(RUN_FLOW_EVENT, onOpen)
        return () => window.removeEventListener(RUN_FLOW_EVENT, onOpen)
    }, [])

    const close = () => setRequest(null)
    const projectPath = request?.projectPath && registry[request.projectPath] ? request.projectPath : defaultProjectChoice(registry)

    return (
        <Dialog open={request !== null} onOpenChange={(open) => { if (!open) close() }}>
            <DialogContent
                data-testid="run-flow-dialog"
                className="flex h-[min(85vh,44rem)] flex-col overflow-hidden sm:max-w-2xl"
                showCloseButton={!request?.flowName}
            >
                {request?.flowName ? (
                    <>
                        <DialogTitle className="sr-only">Run a flow</DialogTitle>
                        <Button
                            type="button"
                            variant="link"
                            size="sm"
                            data-testid="run-flow-back"
                            className="h-auto self-start p-0 text-muted-foreground"
                            onClick={() => setRequest({ ...request, flowName: null })}
                        >
                            <ArrowLeft aria-hidden="true" />
                            All flows
                        </Button>
                        <LaunchPanel
                            key={`${projectPath}:${request.flowName}`}
                            target={{ flowName: request.flowName, previewSource: { kind: 'flow', flowName: request.flowName } }}
                            projectPath={projectPath}
                            initialWorkingDirectory={projectPath ?? ''}
                            onLaunched={(runId) => {
                                if (projectPath) void markProjectUsed(projectPath)
                                close()
                                if (runId) openRun(runId)
                                else useStore.getState().setViewMode('runs')
                            }}
                            onClose={close}
                        />
                    </>
                ) : request ? (
                    <FlowChooser
                        projectPath={projectPath}
                        onProjectChange={(next) => setRequest({ ...request, projectPath: next })}
                        onChoose={(flowName) => setRequest({ ...request, projectPath, flowName })}
                    />
                ) : null}
            </DialogContent>
        </Dialog>
    )
}
