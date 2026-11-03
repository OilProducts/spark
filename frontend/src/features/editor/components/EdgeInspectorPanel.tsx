import { useId, type ReactNode } from 'react'
import type { Edge } from '@xyflow/react'

import type { DiagnosticEntry } from '@/store'
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

export function EdgeInspectorPanel({
    selectedEdge,
    selectedEdgeExtensionEntries,
    edgeFieldDiagnostics,
    isForkEdge,
    onPropertyChange,
    onExtensionValueChange,
    onExtensionRemove,
    onExtensionAdd,
    renderFieldDiagnostics,
}: EdgeInspectorPanelProps) {
    const id = useId()
    return (
        <div className="flex-1 overflow-y-auto px-5 pb-5 pt-3">
            <InspectorScaffold
                scopeLabel="Edge"
                title="Properties"
                entityLabel="Edge"
                entityValue={selectedEdge ? `${selectedEdge.source} -> ${selectedEdge.target}` : undefined}
            >
                {!selectedEdge ? (
                    <InspectorEmptyState message="Select an edge on the canvas to inspect and edit its properties." />
                ) : (
                    <div data-testid="edge-structured-form" className="space-y-4">
                        {isForkEdge ? (
                            <div className="space-y-1.5">
                                <Label htmlFor={`${id}-fork-connection`}>Fork Connection</Label>
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
                                    <Label htmlFor={`${id}-decision`}>Decision ID</Label>
                                    <Input
                                        id={`${id}-decision`}
                                        value={(selectedEdge.data?.decision as string) || ''}
                                        onChange={(event) => onPropertyChange('decision', event.target.value)}
                                        className="font-mono text-sm"
                                        placeholder="e.g. approved"
                                    />
                                    {renderFieldDiagnostics('edge', 'decision', edgeFieldDiagnostics, 'edge-field-diagnostics-decision')}
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-description`}>Description</Label>
                                    <Textarea
                                        id={`${id}-description`}
                                        value={(selectedEdge.data?.description as string) || ''}
                                        onChange={(event) => onPropertyChange('description', event.target.value)}
                                        placeholder="What choosing this decision means."
                                    />
                                    {renderFieldDiagnostics('edge', 'description', edgeFieldDiagnostics, 'edge-field-diagnostics-description')}
                                </div>
                                <div className="space-y-1.5">
                                    <Label htmlFor={`${id}-label`}>Display Label</Label>
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
                            reservedKeys={new Set([
                                'kind',
                                'decision',
                                'description',
                                'label',
                            ])}
                        />
                    </div>
                )}
            </InspectorScaffold>
        </div>
    )
}
