import { useEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import { ChevronDown, Plus, RefreshCw, Search } from 'lucide-react'

import { useStore } from '@/store'
import { sanitizeFlowId } from '@/lib/canonicalFlowModel'
import { saveFlowContent } from '@/lib/flowPersistence'
import { useDialogController } from '@/components/app/dialog-controller'
import { Button } from '@/components/ui/button'
import { InlineError } from '@/components/app/inline-error'
import { FlowTree, type FlowLabel } from '@/components/app/flow-tree'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'
import { projectLabel } from '@/features/projects/model/projectChoices'
import { DefinitionBadges } from '@/features/roles/RolesPanel'
import { fetchDefinitions, type DefinitionItem } from '@/features/roles/rolesModel'
import { flowTitle } from '@/features/runs/model/runOverviewModel'

import { deleteFlowCatalogEntry, loadFlowCatalog, loadFlowSummaries } from './services/flowCatalog'

// A typed flow name as a portable path: "ops/Nightly Audit" is saved as ops/nightly-audit.
export const flowPathFrom = (text: string) => text.split('/')
    .map((part) => part.trim().toLowerCase().replace(/[^a-z0-9_]+/g, '-').replace(/^-+|-+$/g, ''))
    .filter(Boolean)
    .join('/')

// Its title is the name as typed, or a readable form of a path-like name: "nightly-audit" reads "Nightly audit".
const flowTitleFrom = (text: string) => {
    const leaf = text.split('/').map((part) => part.trim()).filter(Boolean).pop() ?? text.trim()
    if (leaf !== flowPathFrom(leaf)) return leaf
    const words = leaf.replace(/[-_]+/g, ' ')
    return words.charAt(0).toUpperCase() + words.slice(1)
}

/** The installed flows, grouped by folder, for the Flows panel. */
function useFlowCatalogPanel() {
    const { alert, confirm, prompt } = useDialogController()
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const activeFlow = useStore((state) => state.activeFlow)
    const setActiveFlow = useStore((state) => state.setActiveFlow)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const [flows, setFlows] = useState<string[]>([])
    // Badge metadata and the project it describes; another project's never shows.
    const [definitions, setDefinitions] = useState<{ project: string | null; flows: Record<string, DefinitionItem> }>({ project: null, flows: {} })
    const flowDefinitions = definitions.project === activeProjectPath ? definitions.flows : {}
    // Titles and descriptions, a hint like the badges: a project whose list fails shows file names.
    const [summaries, setSummaries] = useState<{ project: string | null; labels: Record<string, FlowLabel> }>({ project: null, labels: {} })
    const flowLabels = summaries.project === activeProjectPath ? summaries.labels : {}
    const [isRefreshingFlows, setIsRefreshingFlows] = useState(false)
    // The last load failed: the list may be empty or stale, not the project's flows.
    const [loadFailed, setLoadFailed] = useState(false)
    const activeFlowRef = useRef(activeFlow)
    const isMountedRef = useRef(true)
    const refreshRequestIdRef = useRef(0)
    activeFlowRef.current = activeFlow

    // Titles and descriptions; only the latest request's answer shows.
    const summaryRequestIdRef = useRef(0)
    const refreshSummaries = () => {
        const requestId = summaryRequestIdRef.current + 1
        summaryRequestIdRef.current = requestId
        const project = activeProjectPath
        loadFlowSummaries(project).then(
            (list) => {
                if (isMountedRef.current && requestId === summaryRequestIdRef.current) {
                    setSummaries({ project, labels: Object.fromEntries(list.map((flow) => [flow.name, { title: flow.error ? flowTitle(flow.name) : flow.title, description: flow.description, error: flow.error }])) })
                }
            },
            () => undefined,
        )
    }

    const refreshFlows = async () => {
        const requestId = refreshRequestIdRef.current + 1
        refreshRequestIdRef.current = requestId
        if (isMountedRef.current) {
            setIsRefreshingFlows(true)
        }

        // Badges are a hint: a project whose definition list fails shows none.
        if (activeProjectPath) {
            const project = activeProjectPath
            const settle = (flows: Record<string, DefinitionItem>) => {
                if (isMountedRef.current && requestId === refreshRequestIdRef.current) setDefinitions({ project, flows })
            }
            fetchDefinitions(project).then(
                (list) => settle(Object.fromEntries(list.items.filter((item) => item.kind === 'flow').map((item) => [item.name, item]))),
                (error) => { console.error(error); settle({}) },
            )
        }

        refreshSummaries()

        try {
            const data = await loadFlowCatalog(activeProjectPath)
            if (!isMountedRef.current || requestId !== refreshRequestIdRef.current) {
                return
            }
            setFlows(data)
            setLoadFailed(false)

            const selectedFlow = activeFlowRef.current
            if (selectedFlow && !data.includes(selectedFlow)) {
                setActiveFlow(null)
            }
        } catch (error) {
            console.error(error)
            if (isMountedRef.current && requestId === refreshRequestIdRef.current) {
                setLoadFailed(true)
            }
        } finally {
            if (isMountedRef.current && requestId === refreshRequestIdRef.current) {
                setIsRefreshingFlows(false)
            }
        }
    }

    useEffect(() => {
        isMountedRef.current = true
        void refreshFlows()

        return () => {
            isMountedRef.current = false
        }
    }, [activeProjectPath])

    // A save can retitle the open flow, so its title is read afresh; a flow listed with errors that
    // now saves has been fixed, so the whole list is.
    const savedVersion = useStore((state) => (state.saveState === 'saved' ? state.saveStateVersion : null))
    const activeFlowHasErrors = Boolean(activeFlow && flowLabels[activeFlow]?.error)
    useEffect(() => {
        if (savedVersion === null) return
        if (activeFlowHasErrors) {
            void refreshFlows()
        } else {
            refreshSummaries()
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps -- only a new save refreshes
    }, [savedVersion])

    const createNewFlow = async () => {
        const typed = await prompt({
            title: 'Create flow',
            description: 'Name the flow. Put it in a folder with a slash, such as ops/Nightly audit.',
            label: 'Flow name',
            placeholder: 'ops/Nightly audit',
            confirmLabel: 'Create',
            requireInput: true,
            check: (value) => {
                const path = flowPathFrom(value)
                if (!path) return { ok: false, message: 'Use at least one letter or number.' }
                if (flows.includes(path)) return { ok: false, message: `A flow named ${path} already exists. Pick another name, or open it from the list.` }
                return { ok: true, message: `Saved as ${path}` }
            },
        })
        const fileName = typed ? flowPathFrom(typed) : ''
        if (!fileName || flows.includes(fileName)) return;

        const flowId = sanitizeFlowId(fileName);
        // A minimal FlowDefinition; empty defaults drop out of the JSON.
        const flow = {
            schema_version: '2',
            id: flowId,
            title: flowTitleFrom(typed!),
            description: '',
            goal: '',
            defaults: {
                llm_model: uiDefaults.llm_model || undefined,
                llm_provider: uiDefaults.llm_provider || undefined,
                llm_profile: uiDefaults.llm_profile || undefined,
                reasoning_effort: uiDefaults.reasoning_effort || undefined,
            },
            nodes: {
                start: {
                    kind: 'start',
                    label: 'Start',
                    config: { kind: 'start' },
                    routes: { next: { to: 'end', description: 'Enter the first workflow step.' } },
                },
                end: { kind: 'exit', label: 'End', config: { kind: 'exit', result: { name: 'completed', outcome: 'success' } } },
            },
        }

        const saved = await saveFlowContent(fileName, { flow })
        if (!saved) return

        await refreshFlows();
        setActiveFlow(fileName);
    }

    const handleDeleteFlow = async (e: MouseEvent, fileName: string) => {
        e.stopPropagation();
        const copy = flowDefinitions[fileName]?.layer === 'project_copy'
        const confirmed = await confirm({
            title: copy ? 'Delete your copy of this flow?' : 'Delete flow?',
            description: copy
                ? `${fileName} goes back to the flow bundled with Spark; your changes to it are lost.`
                : `${fileName} is deleted from the project. This can’t be undone.`,
            confirmLabel: 'Delete',
            cancelLabel: 'Keep flow',
            confirmVariant: 'destructive',
        })
        if (!confirmed) return;

        try {
            await deleteFlowCatalogEntry(fileName, activeProjectPath);
        } catch (error) {
            await alert({
                title: 'Flow not deleted',
                description: error instanceof Error ? error.message : String(error),
            })
            return;
        }

        if (activeFlow === fileName) {
            // Deleting it already discarded its edits: close it without asking again.
            useStore.setState({ activeFlow: null });
        }
        await refreshFlows();
    };

    return { activeProjectPath, activeFlow, setActiveFlow, flows, flowDefinitions, flowLabels, isRefreshingFlows, loadFailed, refreshFlows, createNewFlow, handleDeleteFlow }
}

export function FlowsPanelActions({ catalog }: { catalog: ReturnType<typeof useFlowCatalogPanel> }) {
    return (
        <>
            <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label="Refresh flows"
                title="Refresh flows"
                data-testid="editor-flow-refresh-button"
                onClick={() => {
                    void catalog.refreshFlows()
                }}
                disabled={catalog.isRefreshingFlows}
            >
                <RefreshCw aria-hidden="true" className={catalog.isRefreshingFlows ? 'animate-spin' : undefined} />
            </Button>
            <Button
                type="button"
                variant="outline"
                size="sm"
                aria-label="New flow"
                onClick={() => {
                    void catalog.createNewFlow()
                }}
            >
                <Plus aria-hidden="true" />
                New
            </Button>
        </>
    )
}

export function FlowsPanelList({ catalog }: { catalog: ReturnType<typeof useFlowCatalogPanel> }) {
    const registry = useStore((state) => state.projectRegistry)
    const projectFlows = useMemo(
        () => new Set(Object.values(catalog.flowDefinitions).filter((item) => item.layer !== 'bundled').map((item) => item.name)),
        [catalog.flowDefinitions],
    )
    const project = catalog.activeProjectPath
    const [searchQuery, setSearchQuery] = useState('')
    const query = searchQuery.trim().toLowerCase()
    // Every word of the search is in the flow's title, description or name, as Run a flow matches.
    const shownFlows = query
        ? catalog.flows.filter((flow) => {
            const text = `${catalog.flowLabels[flow]?.title ?? ''} ${catalog.flowLabels[flow]?.description ?? ''} ${flow}`.toLowerCase()
            return query.split(/\s+/).every((word) => text.includes(word))
        })
        : catalog.flows
    return (
        <div data-testid="flow-browser-panel" className="space-y-3">
            {/* Flows resolve per project: its own first, then bundled, so the list says whose it is. */}
            <div className="flex min-w-0 items-center gap-2 px-3 text-sm text-muted-foreground">
                <span className="shrink-0">Flows for</span>
                <ProjectPicker
                    heading="Show the flows of"
                    testId="flows-project-picker"
                    defaultProjectPath={project}
                    onPick={(path) => useStore.getState().setActiveProjectPath(path)}
                >
                    <Button type="button" variant="outline" size="sm" data-testid="flows-project-button" className="min-w-0 max-w-full" title={project ?? undefined} aria-label={`Flows for project ${project ? projectLabel(registry, project) : '(none chosen)'}; change project`}>
                        <span className="truncate">{project ? projectLabel(registry, project) : 'Choose a project'}</span>
                        <ChevronDown aria-hidden="true" className="text-muted-foreground" />
                    </Button>
                </ProjectPicker>
            </div>
            {catalog.loadFailed ? (
                <div className="px-3">
                    <InlineError dense data-testid="flow-list-error">
                        {catalog.flows.length > 0 ? 'Couldn’t refresh the flows; this list may be out of date.' : 'Couldn’t load the flows.'}
                        <button type="button" onClick={() => void catalog.refreshFlows()} disabled={catalog.isRefreshingFlows} className="ml-1 font-medium underline underline-offset-4">Try again</button>
                    </InlineError>
                </div>
            ) : null}
            {catalog.flows.length > 0 ? (
                <div className="px-3">
                    <label className="flex h-8 items-center gap-2 rounded-lg border border-border bg-background px-2.5 text-muted-foreground focus-within:ring-2 focus-within:ring-ring">
                        <Search aria-hidden="true" className="size-3.5 shrink-0" />
                        <input
                            type="search"
                            value={searchQuery}
                            onChange={(event) => setSearchQuery(event.target.value)}
                            placeholder="Search flows"
                            aria-label="Search flows by title, name or description"
                            data-testid="flow-list-search-input"
                            className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
                        />
                    </label>
                </div>
            ) : null}
            {query && shownFlows.length === 0 ? (
                <p data-testid="flow-list-search-empty" className="px-3 text-xs text-muted-foreground">No flows match the search.</p>
            ) : null}
            <FlowTree
                dataTestId="editor-flow-tree"
                flows={shownFlows}
                projectFlows={projectFlows}
                projectLabel={catalog.activeProjectPath ? projectLabel(registry, catalog.activeProjectPath) : undefined}
                flowLabels={catalog.flowLabels}
                selectedFlow={catalog.activeFlow}
                // Picking the open flow again leaves its selected step or route, as clicking empty canvas does.
                onSelectFlow={(flow) => flow === catalog.activeFlow
                    ? useStore.setState({ selectedNodeId: null, selectedEdgeId: null })
                    : catalog.setActiveFlow(flow)}
                onDeleteFlow={catalog.handleDeleteFlow}
                // Bundled flows can't be deleted; a project's copy of one can, which brings the bundled flow back.
                canDeleteFlow={(flow) => catalog.flowDefinitions[flow]?.layer !== 'bundled'}
                // The project heading already says a flow is the project's; only a customized bundled flow needs a mark.
                renderFlowIndicator={(flow) => catalog.flowDefinitions[flow]?.layer === 'project_copy'
                    && <span className="flex shrink-0 gap-1"><DefinitionBadges item={catalog.flowDefinitions[flow]} /></span>}
            />
        </div>
    )
}

export { useFlowCatalogPanel }
