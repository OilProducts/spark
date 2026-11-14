import { useId, type ReactNode } from 'react'
import type { Edge } from '@xyflow/react'

import type { DiagnosticEntry } from '@/store'
import { EDGE_CORE_ATTR_KEYS } from '@/lib/canonicalFlowModel'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
import { AdvancedKeyValueEditor } from './AdvancedKeyValueEditor'
import { InspectorEmptyState, InspectorScaffold } from './InspectorScaffold'

type ExtensionEntry = {
    key: string
    value: string
}

interface EdgeInspectorPanelProps {
    selectedEdge: Edge | undefined
    selectedEdgeExtensionEntries: ExtensionEntry[]
    edgeFieldDiagnostics: Record<string, DiagnosticEntry[]>
    isForkEdge: boolean
    /** A step's title, as the canvas shows it. */
    stepTitle: (nodeId: string) => string
    onOpenStep: (nodeId: string) => void
    onPropertyChange: (key: string, value: string | boolean) => void
    onExtensionValueChange: (key: string, value: string) => void
    onExtensionRemove: (key: string) => void
    onExtensionAdd: (key: string, value: string) => void
    renderFieldDiagnostics: (
        scope: 'node' | 'edge',
        field: string,
        fieldDiagnostics: Record<string, DiagnosticEntry[]>,
        testId: string,
    ) => ReactNode
}

/** Marks a field a route can't save without; the field itself says so with aria-required. */
const RequiredMark = () => <span aria-hidden="true" className="text-destructive">*</span>

export function EdgeInspectorPanel({
    selectedEdge,
    selectedEdgeExtensionEntries,
    edgeFieldDiagnostics,
    isForkEdge,
    stepTitle,
    onOpenStep,
    onPropertyChange,
    onExtensionValueChange,
    onExtensionRemove,
    onExtensionAdd,
    renderFieldDiagnostics,
}: EdgeInspectorPanelProps) {
    const id = useId()
    return (
        <div className="flex-1 overflow-y-auto p-4">
            <InspectorScaffold
                scopeLabel="Edge"
                title="Route settings"
                entityLabel="Route"
                entityValue={selectedEdge ? `${selectedEdge.source} -> ${selectedEdge.target}` : undefined}
            >
                {!selectedEdge ? (
                    <InspectorEmptyState message="Select a route on the canvas to inspect and edit it." />
                ) : (
                    <div data-testid="edge-structured-form" className="space-y-4">
                        <p className="text-sm text-muted-foreground">
                            From{' '}
                            <Button type="button" variant="link" className="inline h-auto whitespace-normal p-0 text-left" onClick={() => onOpenStep(selectedEdge.source)}>
                                {stepTitle(selectedEdge.source)}
                            </Button>
                            {' '}to{' '}
                            <Button type="button" variant="link" className="inline h-auto whitespace-normal p-0 text-left" onClick={() => onOpenStep(selectedEdge.target)}>
                                {stepTitle(selectedEdge.target)}
                            </Button>
                        </p>
                        {isForkEdge ? (
                            <div className="space-y-1.5">
                                <Label htmlFor={`${id}-fork-connection`}>Fork connection</Label>
                                <NativeSelect
                                    id={`${id}-fork-connection`}
                                    value={selectedEdge.data?.kind === 'join' ? 'join' : 'branch'}
                                    onChange={(event) => onPropertyChange('kind', event.target.value)}
                                >
                                    <option value="branch">Branch</option>
                                    <option value="join">Join</option>
                                </NativeSelect>
                            </div>
                        ) : (
                            <>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-decision`}>Decision ID<RequiredMark /></Label>
                                    <Input
                                        id={`${id}-decision`}
                                        aria-required
                                        value={(selectedEdge.data?.decision as string) || ''}
                                        onChange={(event) => onPropertyChange('decision', event.target.value)}
                                        className="font-mono text-sm"
                                        placeholder="e.g. approved"
                                    />
                                    {renderFieldDiagnostics('edge', 'decision', edgeFieldDiagnostics, 'edge-field-diagnostics-decision')}
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-description`}>Description<RequiredMark /></Label>
                                    <Textarea
                                        id={`${id}-description`}
                                        aria-required
                                        value={(selectedEdge.data?.description as string) || ''}
                                        onChange={(event) => onPropertyChange('description', event.target.value)}
                                        placeholder="What choosing this decision means."
                                    />
                                    {renderFieldDiagnostics('edge', 'description', edgeFieldDiagnostics, 'edge-field-diagnostics-description')}
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-label`}>Display label</Label>
                                    <Input
                                        id={`${id}-label`}
                                        value={(selectedEdge.data?.label as string) || ''}
                                        onChange={(event) => onPropertyChange('label', event.target.value)}
                                        placeholder="e.g. Approve"
                                    />
                                </div>
                            </>
                        )}
                        <AdvancedKeyValueEditor
                            testIdPrefix="edge"
                            entries={selectedEdgeExtensionEntries}
                            onValueChange={onExtensionValueChange}
                            onRemove={onExtensionRemove}
                            onAdd={onExtensionAdd}
                            reservedKeys={EDGE_CORE_ATTR_KEYS}
                        />
                    </div>
                )}
            </InspectorScaffold>
        </div>
    )
}
