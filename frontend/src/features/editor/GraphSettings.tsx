import { useEffect, useMemo, useRef, useState } from 'react'
import { useNodes, useReactFlow } from '@xyflow/react'
import { useStore, type DiagnosticEntry } from '@/store'
import { generateDot } from '@/lib/dotUtils'
import { extractDebugErrorSummary, recordFlowLoadDebug } from '@/lib/flowLoadDebug'
import { getToolHookCommandWarning } from '@/lib/graphAttrValidation'
import { resolveGraphFieldDiagnostics } from '@/lib/inspectorFieldDiagnostics'
import { resolveModelStylesheetPreview } from '@/lib/modelStylesheetPreview'
import { toExtensionAttrEntries } from '@/lib/extensionAttrs'
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { Button } from '@/ui'
import { InspectorScaffold } from './components/InspectorScaffold'
import {
    parseLaunchInputDefinitions,
    serializeLaunchInputDefinitions,
    validateLaunchInputDefinitions,
    type LaunchInputDefinition,
} from '@/lib/flowContracts'
import {
    CORE_GRAPH_ATTR_KEYS,
    FLOW_LAUNCH_POLICY_LABELS,
    GraphAdvancedAttrsSection,
    GraphExecutionDefaultsSection,
    GraphLaunchPolicySection,
    GraphLaunchInputsSection,
    GraphLlmDefaultsSection,
    GraphMetadataSection,
    GraphRunConfigurationSection,
} from './components/graph-settings/GraphSettingsSections'
import {
    loadGraphLaunchPolicy,
    saveGraphLaunchPolicy,
    type FlowLaunchPolicy,
} from './services/graphLaunchPolicy'

interface GraphSettingsProps {
    inline?: boolean
}

export function GraphSettings({ inline = false }: GraphSettingsProps) {
    const activeFlow = useStore((state) => state.activeFlow)
    const diagnostics = useStore((state) => state.diagnostics)
    const graphAttrs = useStore((state) => state.graphAttrs)
    const graphAttrErrors = useStore((state) => state.graphAttrErrors)
    const graphAttrsUserEditVersion = useStore((state) => state.graphAttrsUserEditVersion)
    const setGraphAttrs = useStore((state) => state.setGraphAttrs)
    const updateGraphAttr = useStore((state) => state.updateGraphAttr)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const viewMode = useStore((state) => state.viewMode)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const editorGraphSettingsPanelOpenByFlow = useStore((state) => state.editorGraphSettingsPanelOpenByFlow)
    const setEditorGraphSettingsPanelOpen = useStore((state) => state.setEditorGraphSettingsPanelOpen)
    const editorShowAdvancedGraphAttrsByFlow = useStore((state) => state.editorShowAdvancedGraphAttrsByFlow)
    const setEditorShowAdvancedGraphAttrs = useStore((state) => state.setEditorShowAdvancedGraphAttrs)
    const editorLaunchInputDraftsByFlow = useStore((state) => state.editorLaunchInputDraftsByFlow)
    const editorLaunchInputDraftErrorByFlow = useStore((state) => state.editorLaunchInputDraftErrorByFlow)
    const setEditorLaunchInputDraftState = useStore((state) => state.setEditorLaunchInputDraftState)
    const { getNodes, getEdges, setNodes } = useReactFlow()
    const flowNodes = useNodes()
    const autosaveScopeRef = useRef<string | null>(null)
    const lastHandledGraphAttrsVersionRef = useRef(graphAttrsUserEditVersion)
    const activeFlowRef = useRef<string | null>(activeFlow)
    const [launchPolicy, setLaunchPolicy] = useState<FlowLaunchPolicy>('disabled')
    const [launchPolicySource, setLaunchPolicySource] = useState<FlowLaunchPolicy | null>(null)
    const [launchPolicyEffective, setLaunchPolicyEffective] = useState<FlowLaunchPolicy>('disabled')
    const [launchPolicyLoadState, setLaunchPolicyLoadState] = useState<'idle' | 'loading' | 'ready' | 'error'>('idle')
    const [launchPolicyLoadError, setLaunchPolicyLoadError] = useState<string | null>(null)
    const [launchPolicySaveState, setLaunchPolicySaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
    const [launchPolicySaveError, setLaunchPolicySaveError] = useState<string | null>(null)
    const flowProviderFallback = graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || ''
    const canApplyDefaults = !!activeFlow && viewMode === 'editor'
    const toolHookPreWarning = getToolHookCommandWarning(graphAttrs['tool.hooks.pre'] || '')
    const toolHookPostWarning = getToolHookCommandWarning(graphAttrs['tool.hooks.post'] || '')
    const stylesheetDiagnostics = diagnostics.filter((diag) => diag.rule_id === 'stylesheet_syntax')
    const hasStylesheetValue = Boolean(graphAttrs.model_stylesheet?.trim())
    const showStylesheetFeedback = hasStylesheetValue || stylesheetDiagnostics.length > 0
    const graphFieldDiagnostics = useMemo(() => resolveGraphFieldDiagnostics(diagnostics), [diagnostics])
    const graphExtensionEntries = useMemo(
        () => toExtensionAttrEntries(graphAttrs as Record<string, unknown>, CORE_GRAPH_ATTR_KEYS),
        [graphAttrs],
    )
    const rawLaunchInputsValue = typeof graphAttrs['spark.launch_inputs'] === 'string'
        ? graphAttrs['spark.launch_inputs']
        : ''
    const parsedLaunchInputs = useMemo(
        () => parseLaunchInputDefinitions(rawLaunchInputsValue),
        [rawLaunchInputsValue],
    )
    const isOpen = activeFlow ? (editorGraphSettingsPanelOpenByFlow[activeFlow] ?? false) : false
    const showAdvancedGraphAttrs = activeFlow
        ? (editorShowAdvancedGraphAttrsByFlow[activeFlow] ?? false)
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
            return `Workspace launch policy saved as ${FLOW_LAUNCH_POLICY_LABELS[launchPolicy]}.`
        }
        if (launchPolicySource === null) {
            return `No catalog entry yet. Effective policy is ${FLOW_LAUNCH_POLICY_LABELS[launchPolicyEffective]}.`
        }
        return `Effective policy: ${FLOW_LAUNCH_POLICY_LABELS[launchPolicyEffective]}.`
    }, [
        activeFlow,
        launchPolicy,
        launchPolicyEffective,
        launchPolicyLoadError,
        launchPolicyLoadState,
        launchPolicySaveError,
        launchPolicySaveState,
        launchPolicySource,
    ])
    const stylesheetPreview = useMemo(() => {
        return resolveModelStylesheetPreview(
            graphAttrs.model_stylesheet || '',
            flowNodes.map((node) => ({
                id: node.id,
                shape: typeof node.data?.shape === 'string' ? node.data.shape : '',
                class: typeof node.data?.class === 'string' ? node.data.class : '',
                llm_model: typeof node.data?.llm_model === 'string' ? node.data.llm_model : '',
                llm_provider: typeof node.data?.llm_provider === 'string' ? node.data.llm_provider : '',
                reasoning_effort: typeof node.data?.reasoning_effort === 'string' ? node.data.reasoning_effort : '',
            })),
            {
                llm_model: graphAttrs.ui_default_llm_model || uiDefaults.llm_model || '',
                llm_provider: graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || '',
                reasoning_effort: graphAttrs.ui_default_reasoning_effort || uiDefaults.reasoning_effort || 'high',
            },
        )
    }, [
        graphAttrs.model_stylesheet,
        graphAttrs.ui_default_llm_model,
        graphAttrs.ui_default_llm_provider,
        graphAttrs.ui_default_reasoning_effort,
        flowNodes,
        uiDefaults.llm_model,
        uiDefaults.llm_provider,
        uiDefaults.reasoning_effort,
    ])

    const { clearPendingSave, saveNow, scheduleSave } = useFlowSaveScheduler<typeof flowNodes>({
        flowName: activeFlow,
        debounceMs: 200,
        buildContent: (nextNodes, currentFlowName) => generateDot(
            currentFlowName,
            nextNodes ?? getNodes(),
            getEdges(),
            graphAttrs,
        ),
    })

    const applyDefaultsToNodes = () => {
        if (!activeFlow) return
        const defaultModel = graphAttrs.ui_default_llm_model || uiDefaults.llm_model || ''
        const defaultProvider = graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || ''
        const defaultReasoning = graphAttrs.ui_default_reasoning_effort || uiDefaults.reasoning_effort || ''

        const currentNodes = getNodes()
        if (currentNodes.length === 0) return

        const updatedNodes = currentNodes.map((node) => ({
            ...node,
            data: {
                ...node.data,
                llm_model: defaultModel,
                llm_provider: defaultProvider,
                reasoning_effort: defaultReasoning,
            },
        }))

        setNodes(updatedNodes)
        saveNow(updatedNodes)
    }

    const handleLaunchInputDefinitionsChange = (entries: LaunchInputDefinition[]) => {
        if (!activeFlow) {
            return
        }
        if (entries.length === 0) {
            setEditorLaunchInputDraftState(activeFlow, entries, null)
            updateGraphAttr('spark.launch_inputs', '')
            return
        }
        const validationError = validateLaunchInputDefinitions(entries)
        setEditorLaunchInputDraftState(activeFlow, entries, validationError)
        if (validationError) {
            return
        }
        updateGraphAttr('spark.launch_inputs', serializeLaunchInputDefinitions(entries))
    }

    useEffect(() => {
        activeFlowRef.current = activeFlow
    }, [activeFlow])

    useEffect(() => {
        if (!activeFlow) {
            setLaunchPolicy('disabled')
            setLaunchPolicySource(null)
            setLaunchPolicyEffective('disabled')
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
                })
                const nextLaunchPolicy = response.launch_policy ?? response.effective_launch_policy
                setLaunchPolicy(nextLaunchPolicy)
                setLaunchPolicySource(response.launch_policy)
                setLaunchPolicyEffective(response.effective_launch_policy)
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
                setLaunchPolicyLoadState('error')
                setLaunchPolicyLoadError(error instanceof Error ? error.message : 'Unable to load workspace launch policy.')
            }
        })()

        return () => {
            cancelled = true
        }
    }, [activeFlow])

    const handleLaunchPolicyChange = async (nextPolicy: FlowLaunchPolicy) => {
        if (!activeFlow || launchPolicyLoadState !== 'ready') {
            return
        }
        const flowName = activeFlow
        const previousPolicy = launchPolicy
        const previousSource = launchPolicySource
        const previousEffective = launchPolicyEffective
        setLaunchPolicy(nextPolicy)
        setLaunchPolicySaveState('saving')
        setLaunchPolicySaveError(null)

        try {
            const response = await saveGraphLaunchPolicy(flowName, nextPolicy)
            if (activeFlowRef.current !== flowName) {
                return
            }
            const savedPolicy = response.launch_policy ?? response.effective_launch_policy
            setLaunchPolicy(savedPolicy)
            setLaunchPolicySource(response.launch_policy)
            setLaunchPolicyEffective(response.effective_launch_policy)
            setLaunchPolicySaveState('saved')
        } catch (error) {
            if (activeFlowRef.current !== flowName) {
                return
            }
            setLaunchPolicy(previousPolicy)
            setLaunchPolicySource(previousSource)
            setLaunchPolicyEffective(previousEffective)
            setLaunchPolicySaveState('error')
            setLaunchPolicySaveError(error instanceof Error ? error.message : 'Unable to save workspace launch policy.')
        }
    }

    useEffect(() => {
        if (!activeFlow) {
            autosaveScopeRef.current = null
            lastHandledGraphAttrsVersionRef.current = graphAttrsUserEditVersion
            clearPendingSave()
            return
        }
        const autosaveScope = activeFlow
        if (autosaveScopeRef.current !== autosaveScope) {
            autosaveScopeRef.current = autosaveScope
            lastHandledGraphAttrsVersionRef.current = graphAttrsUserEditVersion
            clearPendingSave()
            return
        }
        if (graphAttrsUserEditVersion === lastHandledGraphAttrsVersionRef.current) {
            return
        }
        lastHandledGraphAttrsVersionRef.current = graphAttrsUserEditVersion
        scheduleSave()
    }, [activeFlow, clearPendingSave, graphAttrsUserEditVersion, scheduleSave])

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

    const handleGraphExtensionValueChange = (key: string, value: string) => {
        setGraphAttrs({
            ...graphAttrs,
            [key]: value,
        })
    }

    const handleGraphExtensionRemove = (key: string) => {
        const nextGraphAttrs = { ...graphAttrs } as Record<string, unknown>
        delete nextGraphAttrs[key]
        setGraphAttrs(nextGraphAttrs)
    }

    const handleGraphExtensionAdd = (key: string, value: string) => {
        setGraphAttrs({
            ...graphAttrs,
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
                    graphAttrs={graphAttrs}
                    updateGraphAttr={updateGraphAttr}
                />
                <GraphLaunchPolicySection
                    activeFlow={activeFlow}
                    launchPolicy={launchPolicy}
                    launchPolicyLoadState={launchPolicyLoadState}
                    launchPolicySaveState={launchPolicySaveState}
                    launchPolicyStatusMessage={launchPolicyStatusMessage}
                    onLaunchPolicyChange={handleLaunchPolicyChange}
                />
                <GraphLaunchInputsSection
                    launchInputDrafts={launchInputDrafts}
                    launchInputDraftError={launchInputDraftError}
                    onLaunchInputDefinitionsChange={handleLaunchInputDefinitionsChange}
                />
                <GraphExecutionDefaultsSection
                    graphAttrs={graphAttrs}
                    graphAttrErrors={graphAttrErrors}
                    renderFieldDiagnostics={renderFieldDiagnostics}
                    updateGraphAttr={updateGraphAttr}
                />
                <GraphAdvancedAttrsSection
                    graphAttrs={graphAttrs}
                    showAdvancedGraphAttrs={showAdvancedGraphAttrs}
                    graphExtensionEntries={graphExtensionEntries}
                    showStylesheetFeedback={showStylesheetFeedback}
                    stylesheetDiagnostics={stylesheetDiagnostics}
                    stylesheetPreview={stylesheetPreview}
                    toolHookPreWarning={toolHookPreWarning}
                    toolHookPostWarning={toolHookPostWarning}
                    renderFieldDiagnostics={renderFieldDiagnostics}
                    updateGraphAttr={updateGraphAttr}
                    setShowAdvancedGraphAttrs={(value) => {
                        if (!activeFlow) {
                            return
                        }
                        setEditorShowAdvancedGraphAttrs(activeFlow, typeof value === 'function'
                            ? value(showAdvancedGraphAttrs)
                            : value)
                    }}
                    onGraphExtensionValueChange={handleGraphExtensionValueChange}
                    onGraphExtensionRemove={handleGraphExtensionRemove}
                    onGraphExtensionAdd={handleGraphExtensionAdd}
                />
                <GraphLlmDefaultsSection
                    canApplyDefaults={canApplyDefaults}
                    flowProviderFallback={flowProviderFallback}
                    graphAttrs={graphAttrs}
                    uiDefaults={uiDefaults}
                    applyDefaultsToNodes={applyDefaultsToNodes}
                    updateGraphAttr={updateGraphAttr}
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
