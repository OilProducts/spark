import { useEffect, useMemo, useRef, useState } from 'react'
import { useNodes, useReactFlow } from '@xyflow/react'
import { useStore, type DiagnosticEntry } from '@/store'
import { generateFlowYaml } from '@/lib/flowYamlUtils'
import { fetchLlmProfiles } from '@/lib/api/llmProfilesApi'
import type { LlmProfileMetadata } from '@/lib/llmSuggestions'
import { extractDebugErrorSummary, recordFlowLoadDebug } from '@/lib/flowLoadDebug'
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
    FLOW_LAUNCH_POLICY_LABELS,
    GraphAdvancedAttrsSection,
    GraphExecutionDefaultsSection,
    GraphLaunchPolicySection,
    GraphLaunchInputsSection,
    GraphLlmDefaultsSection,
    GraphMetadataSection,
    GraphResultSection,
    GraphRunConfigurationSection,
} from './components/graph-settings/GraphSettingsSections'
import {
    loadGraphLaunchPolicy,
    saveGraphLaunchPolicy,
    type FlowExecutionLockResponse,
    type FlowLaunchPolicy,
} from './services/graphLaunchPolicy'
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
    const activeFlowRef = useRef<string | null>(activeFlow)
    const [launchPolicy, setLaunchPolicy] = useState<FlowLaunchPolicy>('disabled')
    const [launchPolicySource, setLaunchPolicySource] = useState<FlowLaunchPolicy | null>(null)
    const [launchPolicyEffective, setLaunchPolicyEffective] = useState<FlowLaunchPolicy>('disabled')
    const [executionLock, setExecutionLock] = useState<FlowExecutionLockResponse | null>(null)
    const [launchPolicyLoadState, setLaunchPolicyLoadState] = useState<'idle' | 'loading' | 'ready' | 'error'>('idle')
    const [launchPolicyLoadError, setLaunchPolicyLoadError] = useState<string | null>(null)
    const [launchPolicySaveState, setLaunchPolicySaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
    const [launchPolicySaveError, setLaunchPolicySaveError] = useState<string | null>(null)
    const [llmProfiles, setLlmProfiles] = useState<LlmProfileMetadata[]>([])
    const flowProviderFallback = flowMetadata.llm_provider || uiDefaults.llm_provider || ''
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

    useEffect(() => {
        void fetchLlmProfiles().then(setLlmProfiles)
    }, [])
    const isOpen = activeFlow ? (editorGraphSettingsPanelOpenByFlow[activeFlow] ?? false) : false
    const showAdvancedFlowMetadata = activeFlow
        ? (editorShowAdvancedFlowMetadataByFlow[activeFlow] ?? false)
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
    const launchPolicyStatusMessage = useMemo(() => {
        if (!activeFlow) {
            return 'Select a flow to manage workspace launch policy.'
        }
        if (launchPolicyLoadState === 'idle' || launchPolicyLoadState === 'loading') {
            return 'Loading workspace launch policy...'
        }
        if (launchPolicyLoadState === 'error') {
            return launchPolicyLoadError || 'Unable to load workspace launch policy.'
        }
        if (launchPolicySaveState === 'saving') {
            return 'Saving workspace launch policy...'
        }
        if (launchPolicySaveState === 'error') {
            return launchPolicySaveError || 'Unable to save workspace launch policy.'
        }
        if (launchPolicySaveState === 'saved') {
            return 'Workspace flow catalog settings saved.'
        }
        if (launchPolicySource === null && executionLock === null) {
            return `No catalog entry yet. Effective policy is ${FLOW_LAUNCH_POLICY_LABELS[launchPolicyEffective]}.`
        }
        return `Effective policy: ${FLOW_LAUNCH_POLICY_LABELS[launchPolicyEffective]}.`
    }, [
        activeFlow,
        executionLock,
        launchPolicy,
        launchPolicyEffective,
        launchPolicyLoadError,
        launchPolicyLoadState,
        launchPolicySaveError,
        launchPolicySaveState,
        launchPolicySource,
    ])
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
        activeFlowRef.current = activeFlow
    }, [activeFlow])

    useEffect(() => {
        if (!activeFlow) {
            setLaunchPolicy('disabled')
            setLaunchPolicySource(null)
            setLaunchPolicyEffective('disabled')
            setExecutionLock(null)
            setLaunchPolicyLoadState('idle')
            setLaunchPolicyLoadError(null)
            setLaunchPolicySaveState('idle')
            setLaunchPolicySaveError(null)
            return
        }

        let cancelled = false
        setLaunchPolicyLoadState('loading')
        setLaunchPolicyLoadError(null)
        setLaunchPolicySaveState('idle')
        setLaunchPolicySaveError(null)

        void (async () => {
            try {
                recordFlowLoadDebug('launch-policy:request', activeFlow, {
                    source: 'graph-settings',
                })
                const response = await loadGraphLaunchPolicy(activeFlow)
                if (cancelled || activeFlowRef.current !== activeFlow) {
                    return
                }
                recordFlowLoadDebug('launch-policy:response', activeFlow, {
                    source: 'graph-settings',
                    launchPolicy: response.launch_policy ?? null,
                    effectiveLaunchPolicy: response.effective_launch_policy,
                    executionLock: response.execution_lock ?? null,
                })
                const nextLaunchPolicy = response.launch_policy ?? response.effective_launch_policy
                setLaunchPolicy(nextLaunchPolicy)
                setLaunchPolicySource(response.launch_policy)
                setLaunchPolicyEffective(response.effective_launch_policy)
                setExecutionLock(response.execution_lock ?? null)
                setLaunchPolicyLoadState('ready')
            } catch (error) {
                if (cancelled || activeFlowRef.current !== activeFlow) {
                    return
                }
                recordFlowLoadDebug('launch-policy:error', activeFlow, {
                    source: 'graph-settings',
                    ...extractDebugErrorSummary(error),
                })
                setLaunchPolicy('disabled')
                setLaunchPolicySource(null)
                setLaunchPolicyEffective('disabled')
                setExecutionLock(null)
                setLaunchPolicyLoadState('error')
                setLaunchPolicyLoadError(error instanceof Error ? error.message : 'Unable to load workspace launch policy.')
            }
        })()

        return () => {
            cancelled = true
        }
    }, [activeFlow])

    const saveWorkspaceCatalogConfig = async (
        flowName: string,
        nextPolicy: FlowLaunchPolicy,
        nextExecutionLock: FlowExecutionLockResponse | null,
    ) => {
        const response = await saveGraphLaunchPolicy(flowName, {
            launch_policy: nextPolicy,
            execution_lock: nextExecutionLock,
        })
        if (activeFlowRef.current !== flowName) {
            return
        }
        const savedPolicy = response.launch_policy ?? response.effective_launch_policy
        setLaunchPolicy(savedPolicy)
        setLaunchPolicySource(response.launch_policy)
        setLaunchPolicyEffective(response.effective_launch_policy)
        setExecutionLock(response.execution_lock ?? null)
        setLaunchPolicySaveState('saved')
    }

    const handleLaunchPolicyChange = async (nextPolicy: FlowLaunchPolicy) => {
        if (!activeFlow || launchPolicyLoadState !== 'ready') {
            return
        }
        const flowName = activeFlow
        const previousPolicy = launchPolicy
        const previousSource = launchPolicySource
        const previousEffective = launchPolicyEffective
        const previousExecutionLock = executionLock
        setLaunchPolicy(nextPolicy)
        setLaunchPolicySaveState('saving')
        setLaunchPolicySaveError(null)

        try {
            await saveWorkspaceCatalogConfig(flowName, nextPolicy, executionLock)
        } catch (error) {
            if (activeFlowRef.current !== flowName) {
                return
            }
            setLaunchPolicy(previousPolicy)
            setLaunchPolicySource(previousSource)
            setLaunchPolicyEffective(previousEffective)
            setExecutionLock(previousExecutionLock)
            setLaunchPolicySaveState('error')
            setLaunchPolicySaveError(error instanceof Error ? error.message : 'Unable to save workspace launch policy.')
        }
    }

    const handleExecutionLockEnabledChange = (enabled: boolean) => {
        if (!activeFlow || launchPolicyLoadState !== 'ready') {
            return
        }
        const nextExecutionLock = enabled
            ? (executionLock ?? { scope: 'project', key: '', conflict_policy: 'queue' })
            : null
        setExecutionLock(nextExecutionLock)
        if (!enabled || (nextExecutionLock && nextExecutionLock.key.trim())) {
            setLaunchPolicySaveState('saving')
            setLaunchPolicySaveError(null)
            void saveWorkspaceCatalogConfig(activeFlow, launchPolicy, nextExecutionLock).catch((error) => {
                if (activeFlowRef.current !== activeFlow) {
                    return
                }
                setExecutionLock(executionLock)
                setLaunchPolicySaveState('error')
                setLaunchPolicySaveError(error instanceof Error ? error.message : 'Unable to save workspace launch policy.')
            })
        }
    }

    const handleExecutionLockKeyChange = (nextKey: string) => {
        setExecutionLock((current) => (
            current
                ? { ...current, key: nextKey }
                : { scope: 'project', key: nextKey, conflict_policy: 'queue' }
        ))
    }

    const handleExecutionLockKeyCommit = async () => {
        if (!activeFlow || launchPolicyLoadState !== 'ready' || executionLock === null) {
            return
        }
        const previousExecutionLock = executionLock
        if (!executionLock.key.trim()) {
            setLaunchPolicySaveState('error')
            setLaunchPolicySaveError('Execution lock key is required.')
            return
        }
        setLaunchPolicySaveState('saving')
        setLaunchPolicySaveError(null)
        try {
            await saveWorkspaceCatalogConfig(activeFlow, launchPolicy, executionLock)
        } catch (error) {
            if (activeFlowRef.current !== activeFlow) {
                return
            }
            setExecutionLock(previousExecutionLock)
            setLaunchPolicySaveState('error')
            setLaunchPolicySaveError(error instanceof Error ? error.message : 'Unable to save workspace launch policy.')
        }
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
                className="rounded-md border border-border/80 bg-muted/20 px-2 py-1"
            >
                <div className="space-y-1">
                    {diagnosticsForField.map((diag: DiagnosticEntry, index: number) => {
                        const severityClassName = diag.severity === 'error'
                            ? 'text-destructive'
                            : diag.severity === 'warning'
                                ? 'text-amber-800'
                                : 'text-sky-700'
                        return (
                            <p key={`${field}-${diag.rule_id}-${index}`} className={`text-[11px] ${severityClassName}`}>
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
            description="Use the same inspect-edit flow as node and edge inspectors."
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
                <GraphLaunchPolicySection
                    activeFlow={activeFlow}
                    launchPolicy={launchPolicy}
                    executionLock={executionLock}
                    executionLockEnabled={executionLock !== null}
                    launchPolicyLoadState={launchPolicyLoadState}
                    launchPolicySaveState={launchPolicySaveState}
                    launchPolicyStatusMessage={launchPolicyStatusMessage}
                    onLaunchPolicyChange={handleLaunchPolicyChange}
                    onExecutionLockEnabledChange={handleExecutionLockEnabledChange}
                    onExecutionLockKeyChange={handleExecutionLockKeyChange}
                    onExecutionLockKeyCommit={handleExecutionLockKeyCommit}
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
                    flowProviderFallback={flowProviderFallback}
                    flowMetadata={flowMetadata}
                    uiDefaults={uiDefaults}
                    llmProfiles={llmProfiles}
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
                className="bg-background/90 text-xs font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
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
