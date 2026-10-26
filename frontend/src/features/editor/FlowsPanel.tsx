import { useEffect, useRef, useState, type MouseEvent } from 'react'
import { FilePlus } from 'lucide-react'

import { useStore } from '@/store'
import { sanitizeFlowId } from '@/lib/canonicalFlowModel'
import { saveFlowContent } from '@/lib/flowPersistence'
import { useDialogController } from '@/components/app/dialog-controller'
import { Button } from '@/components/ui/button'
import { FlowTree } from '@/components/app/flow-tree'

import { deleteFlowCatalogEntry, loadFlowCatalog } from './services/flowCatalog'

/** The installed flows, grouped by folder, for the Flows panel. */
function useFlowCatalogPanel() {
    const { alert, confirm, prompt } = useDialogController()
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const activeFlow = useStore((state) => state.activeFlow)
    const setActiveFlow = useStore((state) => state.setActiveFlow)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const [flows, setFlows] = useState<string[]>([])
    const [isRefreshingFlows, setIsRefreshingFlows] = useState(false)
    const activeFlowRef = useRef(activeFlow)
    const isMountedRef = useRef(true)
    const refreshRequestIdRef = useRef(0)
    activeFlowRef.current = activeFlow

    const refreshFlows = async () => {
        const requestId = refreshRequestIdRef.current + 1
        refreshRequestIdRef.current = requestId
        if (isMountedRef.current) {
            setIsRefreshingFlows(true)
        }

        try {
            const data = await loadFlowCatalog(activeProjectPath)
            if (!isMountedRef.current || requestId !== refreshRequestIdRef.current) {
                return
            }
            setFlows(data)

            const selectedFlow = activeFlowRef.current
            if (selectedFlow && !data.includes(selectedFlow)) {
                setActiveFlow(null)
            }
        } catch (error) {
            console.error(error)
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

    const createNewFlow = async () => {
        const name = await prompt({
            title: 'Create flow',
            description: 'Enter a flow path such as demos/demo.',
            label: 'Flow path',
            placeholder: 'demos/demo',
            confirmLabel: 'Create',
            requireInput: true,
        })
        if (!name) return;

        const fileName = name;
        const flowId = sanitizeFlowId(fileName);
        // A minimal FlowDefinition; empty defaults drop out of the JSON.
        const flow = {
            schema_version: '1.0',
            id: flowId,
            title: fileName,
            description: '',
            goal: '',
            defaults: {
                llm_model: uiDefaults.llm_model || undefined,
                llm_provider: uiDefaults.llm_provider || undefined,
                llm_profile: uiDefaults.llm_profile || undefined,
                reasoning_effort: uiDefaults.reasoning_effort || undefined,
            },
            nodes: {
                start: { kind: 'start', label: 'Start', config: { kind: 'start' } },
                end: { kind: 'exit', label: 'End', config: { kind: 'exit' } },
            },
            edges: [{ from: 'start', to: 'end' }],
        }

        const saved = await saveFlowContent(fileName, { flow })
        if (!saved) return

        await refreshFlows();
        setActiveFlow(fileName);
    }

    const handleDeleteFlow = async (e: MouseEvent, fileName: string) => {
        e.stopPropagation();
        const confirmed = await confirm({
            title: 'Delete flow?',
            description: `Are you sure you want to delete ${fileName}? A project copy of a bundled flow reverts to the bundled flow.`,
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
            setActiveFlow(null);
        }
        await refreshFlows();
    };

    return { activeFlow, setActiveFlow, flows, isRefreshingFlows, refreshFlows, createNewFlow, handleDeleteFlow }
}

export function FlowsPanelActions({ catalog }: { catalog: ReturnType<typeof useFlowCatalogPanel> }) {
    return (
        <>
            <Button
                type="button"
                variant="ghost"
                size="xs"
                data-testid="editor-flow-refresh-button"
                onClick={() => {
                    void catalog.refreshFlows()
                }}
                disabled={catalog.isRefreshingFlows}
            >
                {catalog.isRefreshingFlows ? 'Refreshing…' : 'Refresh'}
            </Button>
            <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                onClick={() => {
                    void catalog.createNewFlow()
                }}
                title="New Flow"
            >
                <FilePlus className="size-3.5" />
                <span className="sr-only">Create flow</span>
            </Button>
        </>
    )
}

export function FlowsPanelList({ catalog }: { catalog: ReturnType<typeof useFlowCatalogPanel> }) {
    return (
        <div data-testid="flow-browser-panel">
            <FlowTree
                dataTestId="editor-flow-tree"
                flows={catalog.flows}
                selectedFlow={catalog.activeFlow}
                onSelectFlow={catalog.setActiveFlow}
                onDeleteFlow={catalog.handleDeleteFlow}
            />
        </div>
    )
}

export { useFlowCatalogPanel }
