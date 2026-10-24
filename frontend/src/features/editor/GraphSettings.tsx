import { useEffect, useMemo, useRef } from 'react'
import { useNodes, useReactFlow } from '@xyflow/react'
import { useStore, type DiagnosticEntry } from '@/store'
import { generateFlowYaml } from '@/lib/flowYamlUtils'
import { resolveGraphFieldDiagnostics } from '@/lib/inspectorFieldDiagnostics'
import { toExtensionAttrEntries } from '@/lib/extensionAttrs'
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { Button } from '@/components/ui/button'
import { InspectorScaffold } from './components/InspectorScaffold'
import {
    parseLaunchInputDefinitions,
    serializeLaunchInputDefinitions,
    validateLaunchInputDefinitions,
    type LaunchInputDefinition,
} from '@/lib/flowContracts'
import {
    CORE_FLOW_METADATA_KEYS,
    GraphAdvancedAttrsSection,
    GraphExecutionDefaultsSection,
    GraphLaunchInputsSection,
    GraphLlmDefaultsSection,
    GraphMetadataSection,
    GraphResultSection,
    GraphRunConfigurationSection,
} from './components/graph-settings/GraphSettingsSections'
import { useEditorGraphBridgeRef } from './EditorGraphBridgeContext'

interface GraphSettingsProps {
    inline?: boolean
}

export function GraphSettings({ inline = false }: GraphSettingsProps) {
    const activeFlow = useStore((state) => state.activeFlow)
    const diagnostics = useStore((state) => state.diagnostics)
    const flowMetadata = useStore((state) => state.flowMetadata)
    const flowMetadataErrors = useStore((state) => state.flowMetadataErrors)
    const flowMetadataUserEditVersion = useStore((state) => state.flowMetadataUserEditVersion)
    const setFlowMetadata = useStore((state) => state.setFlowMetadata)
    const updateFlowMetadata = useStore((state) => state.updateFlowMetadata)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const viewMode = useStore((state) => state.viewMode)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const preferredGraphSettingsOpen = useStore((state) => state.preferredGraphSettingsOpen)
    const preferredAdvancedControls = useStore((state) => state.preferredAdvancedControls)
    const editorGraphSettingsPanelOpenByFlow = useStore((state) => state.editorGraphSettingsPanelOpenByFlow)
    const setEditorGraphSettingsPanelOpen = useStore((state) => state.setEditorGraphSettingsPanelOpen)
    const editorShowAdvancedFlowMetadataByFlow = useStore((state) => state.editorShowAdvancedFlowMetadataByFlow)
    const setEditorShowAdvancedFlowMetadata = useStore((state) => state.setEditorShowAdvancedFlowMetadata)
    const editorLaunchInputDraftsByFlow = useStore((state) => state.editorLaunchInputDraftsByFlow)
    const editorLaunchInputDraftErrorByFlow = useStore((state) => state.editorLaunchInputDraftErrorByFlow)
    const setEditorLaunchInputDraftState = useStore((state) => state.setEditorLaunchInputDraftState)
    const editorGraphBridgeRef = useEditorGraphBridgeRef()
    const { getNodes, getEdges, setNodes } = useReactFlow()
    const readNodes = () => editorGraphBridgeRef?.current?.getNodes() ?? getNodes()
    const readEdges = () => editorGraphBridgeRef?.current?.getEdges() ?? getEdges()
    const updateNodes = (nextNodes: Parameters<typeof setNodes>[0]) =>
        (editorGraphBridgeRef?.current?.setNodes ?? setNodes)(nextNodes)
    const flowNodes = useNodes()
    const autosaveScopeRef = useRef<string | null>(null)
    const lastHandledFlowMetadataVersionRef = useRef(flowMetadataUserEditVersion)
    const canApplyDefaults = !!activeFlow && viewMode === 'editor'
    const graphFieldDiagnostics = useMemo(() => resolveGraphFieldDiagnostics(diagnostics), [diagnostics])
    const flowMetadataExtensionEntries = useMemo(
        () => toExtensionAttrEntries(flowMetadata as Record<string, unknown>, CORE_FLOW_METADATA_KEYS),
        [flowMetadata],
    )
    const rawLaunchInputsValue = typeof flowMetadata.inputs === 'string'
        ? flowMetadata.inputs
        : ''
    const parsedLaunchInputs = useMemo(
        () => parseLaunchInputDefinitions(rawLaunchInputsValue),
        [rawLaunchInputsValue],
    )

    const isOpen = activeFlow ? (editorGraphSettingsPanelOpenByFlow[activeFlow] ?? preferredGraphSettingsOpen) : false
    const showAdvancedFlowMetadata = activeFlow
        ? (editorShowAdvancedFlowMetadataByFlow[activeFlow] ?? preferredAdvancedControls)
        : false
    const launchInputDrafts = activeFlow
        ? (editorLaunchInputDraftsByFlow[activeFlow] ?? parsedLaunchInputs.entries)
        : parsedLaunchInputs.entries
    const launchInputDraftError = activeFlow
        ? (
            Object.prototype.hasOwnProperty.call(editorLaunchInputDraftErrorByFlow, activeFlow)
                ? editorLaunchInputDraftErrorByFlow[activeFlow]
                : parsedLaunchInputs.error
        )
        : parsedLaunchInputs.error
    const { clearPendingSave, saveNow, scheduleSave } = useFlowSaveScheduler<typeof flowNodes>({
        flowName: activeFlow,
        debounceMs: 200,
        buildContent: (nextNodes, currentFlowName) => generateFlowYaml(
            currentFlowName,
            nextNodes ?? readNodes(),
            readEdges(),
            flowMetadata,
        ),
    })

    const applyDefaultsToNodes = () => {
        if (!activeFlow) return
        const defaultModel = flowMetadata.llm_model || uiDefaults.llm_model || ''
        const defaultProfile = flowMetadata.llm_profile || uiDefaults.llm_profile || ''
        const defaultProvider = defaultProfile ? '' : (flowMetadata.llm_provider || uiDefaults.llm_provider || '')
        const defaultReasoning = flowMetadata.reasoning_effort || uiDefaults.reasoning_effort || ''
        const defaultThinking = flowMetadata.thinking || uiDefaults.thinking || ''

        const currentNodes = readNodes()
        if (currentNodes.length === 0) return

        const updatedNodes = currentNodes.map((node) => ({
            ...node,
            data: {
                ...node.data,
                llm_model: defaultModel,
                llm_provider: defaultProvider,
                llm_profile: defaultProfile,
                reasoning_effort: defaultReasoning,
                thinking: defaultThinking,
                thinking_budget_tokens: defaultThinking === 'budget'
                    ? flowMetadata.thinking_budget_tokens || uiDefaults.thinking_budget_tokens || ''
                    : '',
                reasoning_mode: flowMetadata.reasoning_mode || uiDefaults.reasoning_mode || '',
                reasoning_summary: flowMetadata.reasoning_summary || uiDefaults.reasoning_summary || '',

            },
        }))

        updateNodes(updatedNodes)
        saveNow(updatedNodes)
    }

    const handleLaunchInputDefinitionsChange = (entries: LaunchInputDefinition[]) => {
        if (!activeFlow) {
            return
        }
        if (entries.length === 0) {
            setEditorLaunchInputDraftState(activeFlow, entries, null)
            updateFlowMetadata('inputs', '')
            return
        }
        const validationError = validateLaunchInputDefinitions(entries)
        setEditorLaunchInputDraftState(activeFlow, entries, validationError)
        if (validationError) {
            return
        }
        updateFlowMetadata('inputs', serializeLaunchInputDefinitions(entries))
    }

    useEffect(() => {
        if (!activeFlow) {
            autosaveScopeRef.current = null
            lastHandledFlowMetadataVersionRef.current = flowMetadataUserEditVersion
            clearPendingSave()
            return
        }
        const autosaveScope = activeFlow
        if (autosaveScopeRef.current !== autosaveScope) {
            autosaveScopeRef.current = autosaveScope
            lastHandledFlowMetadataVersionRef.current = flowMetadataUserEditVersion
            clearPendingSave()
            return
        }
        if (flowMetadataUserEditVersion === lastHandledFlowMetadataVersionRef.current) {
            return
        }
        lastHandledFlowMetadataVersionRef.current = flowMetadataUserEditVersion
        scheduleSave()
    }, [
        activeFlow,
        clearPendingSave,
        flowMetadataUserEditVersion,
        scheduleSave,
    ])

    const renderFieldDiagnostics = (field: string, testId: string) => {
        const diagnosticsForField = graphFieldDiagnostics[field] || []
        if (diagnosticsForField.length === 0) {
            return null
        }
        return (
            <div
                data-testid={testId}
                className="rounded-lg bg-muted px-2 py-1"
            >
                <div className="space-y-1">
                    {diagnosticsForField.map((diag: DiagnosticEntry, index: number) => {
                        const severityClassName = diag.severity === 'error'
                            ? 'text-destructive'
                            : diag.severity === 'warning'
                                ? 'text-warning'
                                : 'text-info'
                        return (
                            <p key={`${field}-${diag.rule_id}-${index}`} className={`text-xs ${severityClassName}`}>
                                {diag.message}
                            </p>
                        )
                    })}
                </div>
            </div>
        )
    }

    const handleFlowMetadataExtensionValueChange = (key: string, value: string) => {
        setFlowMetadata({
            ...flowMetadata,
            [key]: value,
        })
    }

    const handleFlowMetadataExtensionRemove = (key: string) => {
        const nextFlowMetadata = { ...flowMetadata } as Record<string, unknown>
        delete nextFlowMetadata[key]
        setFlowMetadata(nextFlowMetadata)
    }

    const handleFlowMetadataExtensionAdd = (key: string, value: string) => {
        setFlowMetadata({
            ...flowMetadata,
            [key]: value,
        })
    }

    const inspectorContent = (
        <InspectorScaffold
            scopeLabel="Graph"
            title="Settings"
            entityLabel="Flow"
            entityValue={activeFlow || undefined}
        >
            <div data-testid="graph-structured-form" className="space-y-4">
                <GraphRunConfigurationSection
                    model={model}
                    workingDir={workingDir}
                    setModel={setModel}
                    setWorkingDir={setWorkingDir}
                />
                <GraphMetadataSection
                    flowMetadata={flowMetadata}
                    updateFlowMetadata={updateFlowMetadata}
                />
                <GraphLaunchInputsSection
                    launchInputDrafts={launchInputDrafts}
                    launchInputDraftError={launchInputDraftError}
                    onLaunchInputDefinitionsChange={handleLaunchInputDefinitionsChange}
                />
                <GraphResultSection
                    flowMetadata={flowMetadata}
                    nodes={flowNodes}
                    updateFlowMetadata={updateFlowMetadata}
                />
                <GraphExecutionDefaultsSection
                    flowMetadata={flowMetadata}
                    flowMetadataErrors={flowMetadataErrors}
                    renderFieldDiagnostics={renderFieldDiagnostics}
                    updateFlowMetadata={updateFlowMetadata}
                />
                <GraphAdvancedAttrsSection
                    showAdvancedFlowMetadata={showAdvancedFlowMetadata}
                    flowMetadataExtensionEntries={flowMetadataExtensionEntries}
                    setShowAdvancedFlowMetadata={(value) => {
                        if (!activeFlow) {
                            return
                        }
                        setEditorShowAdvancedFlowMetadata(activeFlow, typeof value === 'function'
                            ? value(showAdvancedFlowMetadata)
                            : value)
                    }}
                    onFlowMetadataExtensionValueChange={handleFlowMetadataExtensionValueChange}
                    onFlowMetadataExtensionRemove={handleFlowMetadataExtensionRemove}
                    onFlowMetadataExtensionAdd={handleFlowMetadataExtensionAdd}
                />
                <GraphLlmDefaultsSection
                    canApplyDefaults={canApplyDefaults}
                    flowMetadata={flowMetadata}
                    uiDefaults={uiDefaults}
                    projectPath={null}
                    applyDefaultsToNodes={applyDefaultsToNodes}
                    updateFlowMetadata={updateFlowMetadata}
                />
            </div>
        </InspectorScaffold>
    )

    if (inline) {
        return inspectorContent
    }

    return (
        <div className="absolute right-4 top-4 z-20 flex flex-col items-end">
            <Button
                onClick={() => {
                    if (!activeFlow) {
                        return
                    }
                    setEditorGraphSettingsPanelOpen(activeFlow, !isOpen)
                }}
                variant="outline"
                size="sm"
                className="bg-background"
            >
                Graph Settings
            </Button>
            {isOpen && (
                <div className="mt-2 w-80 max-h-[calc(100vh-6rem)] overflow-y-auto rounded-md border border-border bg-card p-4 shadow-lg">
                    {inspectorContent}
                </div>
            )}
        </div>
    )
}
