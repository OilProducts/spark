import { LayoutDashboard, RotateCcw } from 'lucide-react'

import { SegmentedControl } from '@/components/app/segmented-control'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'

type EditorCanvasToolbarProps = {
    mode: 'structured' | 'raw'
    childFlowsExpanded: boolean
    rawHandoffPending: boolean
    runDisabledReason: string | null | undefined
    onSelectStructured: () => void
    onSelectYaml: () => void
    onSetChildFlowsExpanded: (expanded: boolean) => void
    onArrange: () => void
    onReset: () => void
    onAddNode: () => void
    /** The project a run starts in unless another is picked. */
    runDefaultProjectPath: string | null
    onRun: (projectPath: string) => void
}

export function EditorCanvasToolbar({
    mode,
    childFlowsExpanded,
    rawHandoffPending,
    runDisabledReason,
    onSelectStructured,
    onSelectYaml,
    onSetChildFlowsExpanded,
    onArrange,
    onReset,
    onAddNode,
    runDefaultProjectPath,
    onRun,
}: EditorCanvasToolbarProps) {
    const isStructured = mode === 'structured'
    const canEditLayout = isStructured && !childFlowsExpanded

    return (
        // ponytail: one nowrap row fits the default sidebar and the stacked layout; scrolls sideways only if the sidebar is dragged very wide.
        <div
            data-testid="editor-canvas-toolbar"
            aria-label="Canvas toolbar"
            className="flex h-11 shrink-0 items-center gap-2 overflow-x-auto border-b bg-background px-3"
        >
            <SegmentedControl
                data-testid="editor-mode-toggle"
                aria-label="View"
                className="shrink-0"
                options={[
                    { key: 'structured', label: 'Structured', pressed: isStructured, disabled: !isStructured && rawHandoffPending, onPress: onSelectStructured },
                    { key: 'yaml', label: 'YAML', pressed: !isStructured, disabled: !isStructured, onPress: onSelectYaml },
                ]}
            />

            {isStructured ? (
                <>
                    <Separator orientation="vertical" className="data-[orientation=vertical]:h-5" />
                    <SegmentedControl
                        data-testid="editor-child-flow-toggle"
                        aria-label="Child flows"
                        className="shrink-0"
                        options={[
                            { key: 'parent', label: 'Parent', pressed: !childFlowsExpanded, onPress: () => onSetChildFlowsExpanded(false) },
                            { key: 'expanded', label: 'Expanded', pressed: childFlowsExpanded, onPress: () => onSetChildFlowsExpanded(true) },
                        ]}
                    />
                </>
            ) : null}

            {canEditLayout ? (
                <>
                    <Separator orientation="vertical" className="data-[orientation=vertical]:h-5" />
                    <div role="group" aria-label="Layout" className="flex shrink-0 items-center gap-0.5">
                        <Button type="button" size="icon-sm" variant="ghost" aria-label="Auto-arrange layout" title="Auto-arrange layout" onClick={onArrange}>
                            <LayoutDashboard aria-hidden="true" />
                        </Button>
                        <Button type="button" size="icon-sm" variant="ghost" aria-label="Reset saved layout" title="Reset saved layout" onClick={onReset}>
                            <RotateCcw aria-hidden="true" />
                        </Button>
                    </div>
                </>
            ) : null}

            {isStructured ? (
                <div role="group" aria-label="Actions" className="ml-auto flex shrink-0 items-center gap-2">
                    {canEditLayout ? <Button type="button" size="sm" variant="outline" onClick={onAddNode}>+ Node</Button> : null}
                    <ProjectPicker heading="Run in" testId="run-flow-project-picker" defaultProjectPath={runDefaultProjectPath} onPick={onRun}>
                        <Button data-testid="editor-run-button" type="button" size="sm" variant="outline" disabled={Boolean(runDisabledReason)} title={runDisabledReason ?? undefined}>
                            Run…
                        </Button>
                    </ProjectPicker>
                </div>
            ) : null}
        </div>
    )
}
