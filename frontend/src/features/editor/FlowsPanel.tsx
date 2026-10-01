import { useEffect, useRef, useState, type MouseEvent } from 'react'
import { FilePlus } from 'lucide-react'

import { useStore } from '@/store'
import { sanitizeFlowId } from '@/lib/flowYamlUtils'
import { saveFlowContent } from '@/lib/flowPersistence'
import { useDialogController } from '@/components/app/dialog-controller'
import { Button } from '@/components/ui/button'
import { FlowTree } from '@/components/app/flow-tree'

import { deleteFlowCatalogEntry, loadFlowCatalog } from './services/flowCatalog'

/** The installed flows, grouped by folder, for the Flows panel. */
function useFlowCatalogPanel() {
    const { confirm, prompt } = useDialogController()
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
            const data = await loadFlowCatalog()
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
    }, [])

    const createNewFlow = async () => {
        const name = await prompt({
            title: 'Create flow',
            description: 'Enter a flow path such as demos/demo.yaml.',
            label: 'Flow path',
            placeholder: 'demos/demo.yaml',
            confirmLabel: 'Create',
            requireInput: true,
        })
        if (!name) return;

        const hasYamlExtension = /\.(ya?ml)$/i.test(name)
        const fileName = hasYamlExtension ? name : `${name}.yaml`;
        const flowId = sanitizeFlowId(fileName);
        const quoteYaml = (value: string) => JSON.stringify(value)
        const defaults = [
            uiDefaults.llm_model ? `  llm_model: ${quoteYaml(uiDefaults.llm_model)}` : '',
            uiDefaults.llm_provider ? `  llm_provider: ${quoteYaml(uiDefaults.llm_provider)}` : '',
            uiDefaults.llm_profile ? `  llm_profile: ${quoteYaml(uiDefaults.llm_profile)}` : '',
            uiDefaults.reasoning_effort ? `  reasoning_effort: ${quoteYaml(uiDefaults.reasoning_effort)}` : '',
        ].filter(Boolean)
        const defaultsBlock = defaults.length ? `defaults:\n${defaults.join('\n')}\n` : ''

        const initialContent = `schema_version: "1.0"
id: ${flowId}
title: ${quoteYaml(fileName)}
description: ""
goal: ""
${defaultsBlock}nodes:
  start:
    kind: start
    label: Start
    config:
      kind: start
  end:
    kind: exit
    label: End
    config:
      kind: exit
edges:
  - from: start
    to: end
`

        const saved = await saveFlowContent(fileName, initialContent)
        if (!saved) return

        await refreshFlows();
        setActiveFlow(fileName);
    }

    const handleDeleteFlow = async (e: MouseEvent, fileName: string) => {
        e.stopPropagation();
        const confirmed = await confirm({
            title: 'Delete flow?',
            description: `Are you sure you want to delete ${fileName}?`,
            confirmLabel: 'Delete',
            cancelLabel: 'Keep flow',
            confirmVariant: 'destructive',
        })
        if (!confirmed) return;

        await deleteFlowCatalogEntry(fileName);

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
        <div data-testid="flow-browser-panel" className="px-2">
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
