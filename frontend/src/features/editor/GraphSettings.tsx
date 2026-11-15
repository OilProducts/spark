import { useEffect, useMemo, useRef, useState } from 'react'
import { useNodes, useReactFlow } from '@xyflow/react'
import { useStore, type AppState, type DiagnosticEntry } from '@/store'
import { buildCanonicalFlowModelFromEditorState, buildFlowDefinitionFromCanonicalFlowModel } from '@/lib/canonicalFlowModel'
import { resolveGraphFieldDiagnostics } from '@/lib/inspectorFieldDiagnostics'
import { toExtensionAttrEntries } from '@/lib/extensionAttrs'
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { InspectorScaffold } from './components/InspectorScaffold'
import { normalizeProjectPath } from '@/lib/projectPaths'
import { getHandlerType } from '@/lib/nodeVisibility'
import type { TriggerResponse } from '@/lib/api/triggersApi'
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

/**
 * Whether the open flow shows its Flow settings when nothing is selected: the choice last made for it,
 * else the "Open graph settings panel" preference.
 */
export const selectFlowSettingsOpen = (state: AppState) => (
    state.activeFlow ? (state.editorGraphSettingsPanelOpenByFlow[state.activeFlow] ?? state.preferredGraphSettingsOpen) : false
)

/**
 * The triggers that start this flow in this project but leave a required launch input unset,
 * with the inputs' names: each of their firings fails until the trigger sets them.
 */
function triggersMissingInputs(triggers: TriggerResponse[], flow: string | null, projectPath: string | null, inputs: LaunchInputDefinition[]) {
    const required = inputs.filter((input) => input.required && input.key.trim())
    if (!flow || !projectPath || required.length === 0) return []
    return triggers.flatMap((trigger) => {
        if (trigger.action.flow_name !== flow || (trigger.action.mode ?? 'static') !== 'static'
            || normalizeProjectPath(trigger.project_path) !== normalizeProjectPath(projectPath)) return []
        const missing = required.filter((input) => {
            const value = trigger.action.static_context?.[input.key]
            return value === undefined || value === null || (typeof value === 'string' && !value.trim())
        })
        return missing.length ? [{ trigger, missing: missing.map((input) => input.label.trim() || input.key) }] : []
    })
}

const isAgentStep = (node: { data?: Record<string, unknown> }) => getHandlerType(node.data?.kind as string) === 'codergen'

export function GraphSettings() {
    const activeFlow = useStore((state) => state.activeFlow)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const triggers = useStore((state) => state.triggersSession.triggers)
    const diagnostics = useStore((state) => state.diagnostics)
    const flowMetadata = useStore((state) => state.flowMetadata)
    const flowMetadataErrors = useStore((state) => state.flowMetadataErrors)
    const flowMetadataUserEditVersion = useStore((state) => state.flowMetadataUserEditVersion)
    const storeSetFlowMetadata = useStore((state) => state.setFlowMetadata)
    const storeUpdateFlowMetadata = useStore((state) => state.updateFlowMetadata)
    const model = useStore((state) => state.model)
    const setModel = useStore((state) => state.setModel)
    const workingDir = useStore((state) => state.workingDir)
    const setWorkingDir = useStore((state) => state.setWorkingDir)
    const viewMode = useStore((state) => state.viewMode)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const preferredAdvancedControls = useStore((state) => state.preferredAdvancedControls)
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
    // Each edit first tells Undo the settings as they were; edits to one field in a row are one.
    const recordFlowEdit = (key: string | null) => editorGraphBridgeRef?.current?.recordFieldEdit(
        key === null ? null : `flow:${key}`, { flowMetadata: useStore.getState().flowMetadata },
    )
    const updateFlowMetadata: typeof storeUpdateFlowMetadata = (key, value) => {
        if (String(useStore.getState().flowMetadata[key] ?? '') !== value) recordFlowEdit(String(key))
        storeUpdateFlowMetadata(key, value)
    }
    const setFlowMetadata = (metadata: typeof flowMetadata, key: string | null) => {
        recordFlowEdit(key)
        storeSetFlowMetadata(metadata)
    }
    const autosaveScopeRef = useRef<string | null>(null)
    const lastHandledFlowMetadataVersionRef = useRef(flowMetadataUserEditVersion)
    const canApplyDefaults = !!activeFlow && viewMode === 'editor'
    const agentStepCount = flowNodes.filter(isAgentStep).length
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
        buildContent: (nextNodes, currentFlowName) => ({
            flow: buildFlowDefinitionFromCanonicalFlowModel(currentFlowName, buildCanonicalFlowModelFromEditorState(currentFlowName, {
                nodes: nextNodes ?? readNodes(),
                edges: readEdges(),
                flowMetadata,
                flow: editorGraphBridgeRef?.current?.getFlow(),
            })),
        }),
    })

    const defaultThinking = flowMetadata.thinking || uiDefaults.thinking || ''
    const defaultProfile = flowMetadata.llm_profile || uiDefaults.llm_profile || ''
    const stepDefaults: Record<string, string> = {
        llm_model: flowMetadata.llm_model || uiDefaults.llm_model || '',
        llm_provider: defaultProfile ? '' : (flowMetadata.llm_provider || uiDefaults.llm_provider || ''),
        llm_profile: defaultProfile,
        reasoning_effort: flowMetadata.reasoning_effort || uiDefaults.reasoning_effort || '',
        thinking: defaultThinking,
        thinking_budget_tokens: defaultThinking === 'budget'
            ? flowMetadata.thinking_budget_tokens || uiDefaults.thinking_budget_tokens || ''
            : '',
        reasoning_mode: flowMetadata.reasoning_mode || uiDefaults.reasoning_mode || '',
        reasoning_summary: flowMetadata.reasoning_summary || uiDefaults.reasoning_summary || '',
    }
    // What the last apply did, shown while the flow and its defaults are as they were then.
    const appliedKey = `${activeFlow}\n${JSON.stringify(stepDefaults)}`
    const [applied, setApplied] = useState<{ key: string; message: string } | null>(null)

    const applyDefaultsToNodes = () => {
        if (!activeFlow) return
        // Only agent steps run a model; other kinds would save settings they never use.
        const currentNodes = readNodes()
        const agentNodes = currentNodes.filter(isAgentStep)
        const changing = agentNodes.filter((node) => Object.entries(stepDefaults).some(([key, value]) => String(node.data?.[key] ?? '') !== value))
        const steps = (count: number) => `${count} agent ${count === 1 ? 'step' : 'steps'}`
        if (changing.length === 0) {
            if (agentNodes.length > 0) setApplied({ key: appliedKey, message: `${agentNodes.length === 1 ? 'Its agent step already uses' : `All ${steps(agentNodes.length)} already use`} these defaults.` })
            return
        }
        editorGraphBridgeRef?.current?.recordFieldEdit(null, { nodes: changing })
        const updatedNodes = currentNodes.map((node) => changing.includes(node) ? { ...node, data: { ...node.data, ...stepDefaults } } : node)
        updateNodes(updatedNodes)
        saveNow(updatedNodes)
        setApplied({ key: appliedKey, message: `Set ${steps(changing.length)} to these defaults.` })
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
        }, key)
    }

    const handleFlowMetadataExtensionRemove = (key: string) => {
        const nextFlowMetadata = { ...flowMetadata } as Record<string, unknown>
        delete nextFlowMetadata[key]
        setFlowMetadata(nextFlowMetadata, null)
    }

    const handleFlowMetadataExtensionAdd = (key: string, value: string) => {
        setFlowMetadata({
            ...flowMetadata,
            [key]: value,
        }, null)
    }

    return (
        <InspectorScaffold
            scopeLabel="Graph"
            title="Flow settings"
            entityLabel="Flow"
            entityValue={activeFlow || undefined}
        >
            <div data-testid="graph-structured-form" className="space-y-4">
                <GraphMetadataSection
                    flowMetadata={flowMetadata}
                    updateFlowMetadata={updateFlowMetadata}
                />
                <GraphLaunchInputsSection
                    launchInputDrafts={launchInputDrafts}
                    launchInputDraftError={launchInputDraftError}
                    triggersMissingInputs={triggersMissingInputs(triggers, activeFlow, activeProjectPath, launchInputDrafts)}
                    onLaunchInputDefinitionsChange={handleLaunchInputDefinitionsChange}
                />
                <GraphResultSection
                    flowMetadata={flowMetadata}
                    nodes={flowNodes}
                    updateFlowMetadata={updateFlowMetadata}
                />
                <GraphRunConfigurationSection
                    model={model}
                    workingDir={workingDir}
                    setModel={setModel}
                    setWorkingDir={setWorkingDir}
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
                    agentStepCount={agentStepCount}
                    appliedMessage={applied?.key === appliedKey ? applied.message : null}
                    flowMetadata={flowMetadata}
                    uiDefaults={uiDefaults}
                    projectPath={null}
                    applyDefaultsToNodes={applyDefaultsToNodes}
                    updateFlowMetadata={updateFlowMetadata}
                />
            </div>
        </InspectorScaffold>
    )
}
