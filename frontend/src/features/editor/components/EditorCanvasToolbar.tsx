import { Box, Boxes, FileCode, LayoutDashboard, RotateCcw, Workflow, type LucideIcon } from 'lucide-react'

import { SegmentedControl } from '@/components/app/segmented-control'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'
import { CanvasViewControls } from '@/features/workflow-canvas'
import type { EditorMode } from '@/state/store-types'
import { cn } from '@/lib/utils'

type EditorCanvasToolbarProps = {
    mode: EditorMode
    /** Set until the flow is loaded: a view switch saves the canvas, which is empty until then. */
    modeDisabled: boolean
    childFlowsExpanded: boolean
    runDisabledReason: string | null | undefined
    onSelectStructured: () => void
    onSelectFiles: () => void
    onSetChildFlowsExpanded: (expanded: boolean) => void
    onArrange: () => void
    onReset: () => void
    onAddNode: () => void
    /** The project a run starts in unless another is picked. */
    runDefaultProjectPath: string | null
    onRun: (projectPath: string) => void
    /** The minimap toggle; omitted where the canvas has no minimap. */
    minimap?: { shown: boolean; onToggle: () => void }
}

// Below 42rem of toolbar the view toggles show only icons (named by aria-label), so the row stays one line beside the flow settings at 1280px.
const segmentLabel = (Icon: LucideIcon, text: string) => ({
    'aria-label': text,
    title: text,
    label: (
        <>
            <Icon aria-hidden="true" className="size-4 @2xl:hidden" />
            <span className="hidden @2xl:inline">{text}</span>
        </>
    ),
})

export function EditorCanvasToolbar({
    mode,
    modeDisabled,
    childFlowsExpanded,
    runDisabledReason,
    onSelectStructured,
    onSelectFiles,
    onSetChildFlowsExpanded,
    onArrange,
    onReset,
    onAddNode,
    runDefaultProjectPath,
    onRun,
    minimap,
}: EditorCanvasToolbarProps) {
    const isStructured = mode === 'structured'
    const canEditLayout = isStructured && !childFlowsExpanded

    return (
        // One row; on a phone-width canvas the actions wrap to a second row rather than scroll out of sight.
        <div
            data-testid="editor-canvas-toolbar"
            aria-label="Canvas toolbar"
            className="@container flex min-h-11 shrink-0 flex-wrap items-center gap-x-1.5 gap-y-2 border-b bg-background px-3 py-1.5"
        >
            <SegmentedControl
                data-testid="editor-mode-toggle"
                aria-label="View"
                className="shrink-0"
                options={[
                    { key: 'structured', ...segmentLabel(Workflow, 'Structured'), pressed: isStructured, onPress: onSelectStructured, disabled: modeDisabled },
                    { key: 'files', ...segmentLabel(FileCode, 'Files'), pressed: !isStructured, onPress: onSelectFiles, disabled: modeDisabled },
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
                            { key: 'parent', ...segmentLabel(Box, 'Parent'), pressed: !childFlowsExpanded, onPress: () => onSetChildFlowsExpanded(false) },
                            { key: 'expanded', ...segmentLabel(Boxes, 'Expanded'), pressed: childFlowsExpanded, onPress: () => onSetChildFlowsExpanded(true) },
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

            {/* The canvas sits beside the files only from md up. */}
            <Separator orientation="vertical" className={cn('data-[orientation=vertical]:h-5', !isStructured && 'max-md:hidden')} />
            <CanvasViewControls minimap={minimap} lockable={canEditLayout} className={isStructured ? undefined : 'max-md:hidden'} />

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
