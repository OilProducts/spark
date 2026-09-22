import type { ReactNode } from 'react'
import type { Edge } from '@xyflow/react'

import type { DiagnosticEntry } from '@/store'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
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
    selectedEdgeConditionDiagnostics: DiagnosticEntry[]
    conditionPreviewHasError: boolean
    conditionPreviewHasWarning: boolean
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
    selectedEdgeConditionDiagnostics,
    conditionPreviewHasError,
    conditionPreviewHasWarning,
    onPropertyChange,
    onExtensionValueChange,
    onExtensionRemove,
    onExtensionAdd,
    renderFieldDiagnostics,
}: EdgeInspectorPanelProps) {
    return (
        <div className="flex-1 overflow-y-auto px-5 pb-5 pt-3">
            <InspectorScaffold
                scopeLabel="Edge"
                title="Properties"
                description="Use the same inspect-edit flow as graph and node inspectors."
                entityLabel="Edge"
                entityValue={selectedEdge ? `${selectedEdge.source} -> ${selectedEdge.target}` : undefined}
            >
                {!selectedEdge ? (
                    <InspectorEmptyState message="Select an edge on the canvas to inspect and edit its properties." />
                ) : (
                    <div data-testid="edge-structured-form" className="space-y-4">
                        <div className="space-y-1.5">
                            <Label>Label</Label>
                            <Input
                                value={(selectedEdge.data?.label as string) || ''}
                                onChange={(event) => onPropertyChange('label', event.target.value)}
                                placeholder="e.g. Approve"
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label>Condition</Label>
                            <Input
                                value={(selectedEdge.data?.condition as string) || ''}
                                onChange={(event) => onPropertyChange('condition', event.target.value)}
                                placeholder='e.g. outcome = "success"'
                            />
                            <div data-testid="edge-condition-syntax-hints" className="space-y-1 rounded-md border border-border/80 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
                                <p>Use && to join clauses.</p>
                                <p>{'Supported keys: outcome, preferred_label, context.<path>'}</p>
                                <p>Operators: = or !=</p>
                            </div>
                            {renderFieldDiagnostics('edge', 'condition', edgeFieldDiagnostics, 'edge-field-diagnostics-condition')}
                            <div
                                data-testid="edge-condition-preview-feedback"
                                className={`rounded-md border px-3 py-2 text-xs ${
                                    conditionPreviewHasError
                                        ? 'border-destructive/40 bg-destructive/10 text-destructive'
                                        : conditionPreviewHasWarning
                                            ? 'border-amber-500/40 bg-amber-500/10 text-amber-800'
                                            : 'border-emerald-500/40 bg-emerald-500/10 text-emerald-700'
                                }`}
                            >
                                {selectedEdgeConditionDiagnostics.length > 0 ? (
                                    <ul className="space-y-1">
                                        {selectedEdgeConditionDiagnostics.map((diag, index) => (
                                            <li key={`${diag.rule_id}-${diag.message}-${index}`}>{diag.message}</li>
                                        ))}
                                    </ul>
                                ) : (
                                    <p>Condition syntax looks valid in preview.</p>
                                )}
                            </div>
                        </div>
                        <div className="space-y-1.5">
                            <Label>Weight</Label>
                            <Input
                                value={(selectedEdge.data?.weight as number | string | undefined) ?? ''}
                                onChange={(event) => onPropertyChange('weight', event.target.value)}
                                placeholder="0"
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label>Fidelity</Label>
                            <Input
                                value={(selectedEdge.data?.fidelity as string) || ''}
                                onChange={(event) => onPropertyChange('fidelity', event.target.value)}
                                placeholder="full | truncate | compact | summary:low"
                            />
                            {renderFieldDiagnostics('edge', 'fidelity', edgeFieldDiagnostics, 'edge-field-diagnostics-fidelity')}
                        </div>
                        <div className="space-y-1.5">
                            <Label>Thread ID</Label>
                            <Input
                                value={(selectedEdge.data?.thread_id as string) || ''}
                                onChange={(event) => onPropertyChange('thread_id', event.target.value)}
                            />
                        </div>
                        <div className="flex items-center gap-2">
                            <Checkbox
                                id="edge-loop-restart"
                                checked={Boolean(selectedEdge.data?.loop_restart)}
                                onCheckedChange={(checked) => onPropertyChange('loop_restart', checked === true)}
                            />
                            <Label htmlFor="edge-loop-restart" className="text-sm font-medium">
                                Loop Restart
                            </Label>
                        </div>
                        <AdvancedKeyValueEditor
                            testIdPrefix="edge"
                            entries={selectedEdgeExtensionEntries}
                            onValueChange={onExtensionValueChange}
                            onRemove={onExtensionRemove}
                            onAdd={onExtensionAdd}
                            reservedKeys={new Set([
                                'label',
                                'condition',
                                'weight',
                                'fidelity',
                                'thread_id',
                                'loop_restart',
                            ])}
                        />
                    </div>
                )}
            </InspectorScaffold>
        </div>
    )
}
