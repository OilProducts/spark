import { LayoutDashboard, RotateCcw } from 'lucide-react'

import { Button } from '@/components/ui/button'

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
        <div
            data-testid="editor-canvas-toolbar"
            aria-label="Canvas toolbar"
            className="flex max-w-[calc(100%-2rem)] flex-wrap items-center gap-1.5 rounded-lg border border-border/70 bg-background/90 p-1.5 shadow-sm backdrop-blur-sm"
        >
            <div data-testid="editor-mode-toggle" role="group" aria-label="View" className="flex shrink-0 items-center gap-0.5">
                <Button type="button" size="sm" variant="ghost" aria-pressed={isStructured} className={segmentClass} disabled={!isStructured && rawHandoffPending} onClick={onSelectStructured}>
                    Structured
                </Button>
                <Button type="button" size="sm" variant="ghost" aria-pressed={!isStructured} className={segmentClass} disabled={!isStructured} onClick={onSelectYaml}>
                    YAML
                </Button>
                {isStructured ? (
                    <span data-testid="editor-child-flow-toggle" className="contents">
                        <span aria-hidden="true" className="mx-1 h-5 w-px bg-border" />
                        <Button type="button" size="sm" variant="ghost" aria-pressed={!childFlowsExpanded} className={segmentClass} onClick={() => onSetChildFlowsExpanded(false)}>
                            Parent
                        </Button>
                        <Button type="button" size="sm" variant="ghost" aria-pressed={childFlowsExpanded} className={segmentClass} onClick={() => onSetChildFlowsExpanded(true)}>
                            Expanded
                        </Button>
                    </span>
                ) : null}
            </div>

            {canEditLayout ? (
                <div role="group" aria-label="Layout" className="flex shrink-0 items-center gap-0.5 border-l border-border pl-1.5">
                    <Button type="button" size="icon-sm" variant="ghost" aria-label="Arrange" title="Arrange" onClick={onArrange}>
                        <LayoutDashboard aria-hidden="true" />
                    </Button>
                    <Button type="button" size="icon-sm" variant="ghost" aria-label="Reset" title="Reset layout" onClick={onReset}>
                        <RotateCcw aria-hidden="true" />
                    </Button>
                </div>
            ) : null}

            {isStructured ? (
                <div role="group" aria-label="Actions" className="flex shrink-0 items-center gap-1 border-l border-border pl-1.5">
                    {canEditLayout ? <Button type="button" size="sm" variant="secondary" onClick={onAddNode}>+ Node</Button> : null}
                    <Button data-testid="editor-run-button" type="button" size="sm" onClick={onRun} disabled={Boolean(runDisabledReason)} title={runDisabledReason ?? undefined}>
                        Run
                    </Button>
                </div>
            ) : null}
        </div>
    )
}
