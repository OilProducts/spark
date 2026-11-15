import { Box, Boxes, FileCode, LayoutDashboard, Play, RotateCcw, SlidersHorizontal, SquarePlus, Undo2, Workflow, type LucideIcon } from 'lucide-react'

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
    /** A view switch is saving the view it leaves. */
    switchingMode?: boolean
    childFlowsExpanded: boolean
    /** Whether the flow has child-flow steps; without them the Child flows views are the same. */
    hasChildFlows?: boolean
    runDisabledReason: string | null | undefined
    onSelectStructured: () => void
    onSelectFiles: () => void
    onSetChildFlowsExpanded: (expanded: boolean) => void
    onArrange: () => void
    onReset: () => void
    onAddNode: () => void
    /** Takes back the latest canvas edit, which `label` names; null when there is none. Omitted where the canvas can't be edited. */
    undo?: { label: string | null; onUndo: () => void }
    /** The project a run starts in unless another is picked. */
    runDefaultProjectPath: string | null
    onRun: (projectPath: string) => void
    /** The minimap toggle; omitted where the canvas has no minimap. */
    minimap?: { shown: boolean; onToggle: () => void }
    /** Shows or hides the flow's settings beside the canvas; `shown` while they are what the inspector shows. */
    flowSettings?: { shown: boolean; onToggle: () => void }
}

const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent)

// Below 42rem of toolbar the toggles and actions show only icons (named by aria-label), so the row stays one line beside the flow settings at 1280px.
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
    switchingMode = false,
    childFlowsExpanded,
    hasChildFlows = true,
    runDisabledReason,
    onSelectStructured,
    onSelectFiles,
    onSetChildFlowsExpanded,
    onArrange,
    onReset,
    onAddNode,
    undo,
    runDefaultProjectPath,
    onRun,
    minimap,
    flowSettings,
}: EditorCanvasToolbarProps) {
    const isStructured = mode === 'structured'
    const canEditLayout = isStructured && !childFlowsExpanded

    return (
        // One row, even on a phone (the zoom buttons drop there); should it still overflow, the actions wrap rather than scroll out of sight.
        <div
            data-testid="editor-canvas-toolbar"
            aria-label="Canvas toolbar"
            className="@container flex min-h-11 shrink-0 flex-wrap items-center gap-x-1 gap-y-2 border-b bg-background px-2 py-1.5 @2xl:gap-x-1.5 @2xl:px-3"
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
            {switchingMode ? <span data-testid="editor-mode-switching" role="status" className="shrink-0 text-xs text-muted-foreground">Saving…</span> : null}

            {/* Expanded stays reachable to leave it, even once the flow has no child flows left. */}
            {isStructured && (hasChildFlows || childFlowsExpanded) ? (
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
                    {flowSettings ? (
                        <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            aria-label="Flow settings"
                            title={flowSettings.shown ? 'Hide flow settings' : 'Show flow settings'}
                            aria-pressed={flowSettings.shown}
                            className="aria-pressed:bg-accent aria-pressed:text-foreground"
                            onClick={flowSettings.onToggle}
                        >
                            <SlidersHorizontal aria-hidden="true" className="@2xl:hidden" />
                            <span className="hidden @2xl:inline">Flow settings</span>
                        </Button>
                    ) : null}
                    {canEditLayout && undo ? (
                        // Also Cmd/Ctrl+Z, which a phone has no keys for.
                        <Button
                            type="button"
                            size="icon-sm"
                            variant="ghost"
                            data-testid="editor-undo"
                            aria-label={undo.label ?? 'Undo'}
                            aria-keyshortcuts="Meta+Z Control+Z"
                            title={undo.label ? `${undo.label} (${isMac ? '⌘Z' : 'Ctrl+Z'})` : 'Nothing to undo'}
                            // Not `disabled`: taking back the last edit would drop focus to the page.
                            aria-disabled={!undo.label || undefined}
                            className="aria-disabled:opacity-50 aria-disabled:hover:bg-transparent"
                            onClick={undo.label ? undo.onUndo : undefined}
                        >
                            <Undo2 aria-hidden="true" />
                        </Button>
                    ) : null}
                    {canEditLayout ? (
                        <Button type="button" size="sm" variant="outline" aria-label="Add step" title="Add step" onClick={onAddNode}>
                            <SquarePlus aria-hidden="true" className="@2xl:hidden" />
                            <span className="hidden @2xl:inline">+ Step</span>
                        </Button>
                    ) : null}
                    {/* A disabled button shows no tooltip, so its wrapper says why Run is off. */}
                    <span title={runDisabledReason ?? undefined} className="inline-flex">
                        <ProjectPicker heading="Run in" testId="run-flow-project-picker" defaultProjectPath={runDefaultProjectPath} onPick={onRun}>
                            <Button data-testid="editor-run-button" type="button" size="sm" variant="outline" aria-label="Run…" disabled={Boolean(runDisabledReason)} aria-describedby={runDisabledReason ? 'editor-run-disabled-reason' : undefined}>
                                <Play aria-hidden="true" className="@2xl:hidden" />
                                <span className="hidden @2xl:inline">Run…</span>
                            </Button>
                        </ProjectPicker>
                    </span>
                    {runDisabledReason ? <span id="editor-run-disabled-reason" className="sr-only">{runDisabledReason}</span> : null}
                </div>
            ) : null}
        </div>
    )
}
