import { LayoutDashboard, RotateCcw } from 'lucide-react'

import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'

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
    onRun: () => void
}

const segmentClass = 'px-3 text-muted-foreground hover:text-foreground aria-pressed:bg-accent aria-pressed:text-foreground'

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
            <div data-testid="editor-mode-toggle" role="group" aria-label="View" className="flex shrink-0 items-center gap-0.5">
                <Button type="button" size="sm" variant="ghost" aria-pressed={isStructured} className={segmentClass} disabled={!isStructured && rawHandoffPending} onClick={onSelectStructured}>
                    Structured
                </Button>
                <Button type="button" size="sm" variant="ghost" aria-pressed={!isStructured} className={segmentClass} disabled={!isStructured} onClick={onSelectYaml}>
                    YAML
                </Button>
            </div>

            {isStructured ? (
                <>
                    <Separator orientation="vertical" className="data-[orientation=vertical]:h-5" />
                    <div data-testid="editor-child-flow-toggle" role="group" aria-label="Child flows" className="flex shrink-0 items-center gap-0.5">
                        <Button type="button" size="sm" variant="ghost" aria-pressed={!childFlowsExpanded} className={segmentClass} onClick={() => onSetChildFlowsExpanded(false)}>
                            Parent
                        </Button>
                        <Button type="button" size="sm" variant="ghost" aria-pressed={childFlowsExpanded} className={segmentClass} onClick={() => onSetChildFlowsExpanded(true)}>
                            Expanded
                        </Button>
                    </div>
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
                    <Button data-testid="editor-run-button" type="button" size="sm" onClick={onRun} disabled={Boolean(runDisabledReason)} title={runDisabledReason ?? undefined}>
                        Run
                    </Button>
                </div>
            ) : null}
        </div>
    )
}
