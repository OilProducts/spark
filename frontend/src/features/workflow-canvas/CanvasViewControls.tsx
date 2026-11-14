import { LocateFixed, Lock, LockOpen, Map as MapIcon, Maximize, Minus, Plus } from 'lucide-react'
import { useReactFlow, useStore, useStoreApi, type ReactFlowState } from '@xyflow/react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { READABLE_ZOOM } from './graphFraming'

type CanvasViewControlsProps = {
    /** The minimap toggle; omitted where a minimap has nothing to show. */
    minimap?: { shown: boolean; onToggle: () => void }
    /** Offer the lock that stops nodes being moved, connected or selected (editable canvases only). */
    lockable?: boolean
    className?: string
}

const isInteractiveSelector = (state: { nodesDraggable: boolean; nodesConnectable: boolean; elementsSelectable: boolean }) => (
    state.nodesDraggable || state.nodesConnectable || state.elementsSelectable
)

// The shown steps as `id` and label pairs, one per line, so moving a step re-renders nothing.
const stepListSelector = (state: ReactFlowState) => state.nodes
    .filter((node) => !node.hidden)
    .map((node) => `${node.id}\t${(typeof node.data?.label === 'string' && node.data.label.trim()) || node.id}`)
    .join('\n')

/**
 * Zoom, fit, minimap and lock buttons for a graph, shown in its toolbar rather than over the
 * canvas, so they never cover a node. Must render inside the graph's ReactFlowProvider.
 */
export function CanvasViewControls({ minimap, lockable = false, className }: CanvasViewControlsProps) {
    const { zoomIn, zoomOut, fitView, getInternalNode, getZoom, setCenter } = useReactFlow()
    const store = useStoreApi()
    const isInteractive = useStore(isInteractiveSelector)
    const stepList = useStore(stepListSelector)
    const steps = stepList ? stepList.split('\n').map((line) => line.split('\t') as [id: string, label: string]) : []
    // A big flow fit whole is too small to read: this finds a step by name and centres it, readable.
    const goToStep = (id: string) => {
        const node = getInternalNode(id)
        if (!node) return
        const { x, y } = node.internals.positionAbsolute
        void setCenter(x + (node.measured.width ?? 0) / 2, y + (node.measured.height ?? 0) / 2, {
            zoom: Math.max(getZoom(), READABLE_ZOOM),
            duration: 200,
        })
    }

    return (
        // On a phone, pinching zooms and a minimap would cover the graph: Fit and Lock stay, so the toolbar keeps to one row.
        <div role="group" aria-label="Zoom" data-testid="canvas-view-controls" className={cn('flex shrink-0 items-center gap-0.5', className)}>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Zoom out" title="Zoom out" className="max-sm:hidden" onClick={() => void zoomOut({ duration: 150 })}>
                <Minus aria-hidden="true" />
            </Button>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Zoom in" title="Zoom in" className="max-sm:hidden" onClick={() => void zoomIn({ duration: 150 })}>
                <Plus aria-hidden="true" />
            </Button>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Fit whole graph" title="Fit whole graph" onClick={() => void fitView({ padding: 0.1, duration: 200 })}>
                <Maximize aria-hidden="true" />
            </Button>
            {/* A native list, shown as an icon: it opens on a click or the keyboard, and typing a step's name picks it. */}
            {steps.length > 1 ? (
                <span className="relative inline-flex size-8 shrink-0 items-center justify-center rounded-lg text-foreground hover:bg-muted has-[select:focus-visible]:ring-2 has-[select:focus-visible]:ring-ring">
                    <LocateFixed aria-hidden="true" className="size-4" />
                    <select
                        aria-label="Go to step"
                        title="Go to step"
                        data-testid="canvas-go-to-step"
                        value=""
                        onChange={(event) => goToStep(event.target.value)}
                        className="absolute inset-0 cursor-pointer appearance-none opacity-0"
                    >
                        <option value="" disabled>Go to step</option>
                        {steps.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
                    </select>
                </span>
            ) : null}
            {lockable ? (
                <Button
                    type="button"
                    size="icon-sm"
                    variant="ghost"
                    aria-label="Lock canvas"
                    title={isInteractive ? 'Lock canvas' : 'Unlock canvas'}
                    aria-pressed={!isInteractive}
                    className="aria-pressed:bg-accent aria-pressed:text-foreground"
                    onClick={() => store.setState({
                        nodesDraggable: !isInteractive,
                        nodesConnectable: !isInteractive,
                        elementsSelectable: !isInteractive,
                    })}
                >
                    {isInteractive ? <LockOpen aria-hidden="true" /> : <Lock aria-hidden="true" />}
                </Button>
            ) : null}
            {minimap ? (
                <Button
                    type="button"
                    size="icon-sm"
                    variant="ghost"
                    aria-label="Minimap"
                    title={minimap.shown ? 'Hide minimap' : 'Show minimap'}
                    aria-pressed={minimap.shown}
                    data-testid="canvas-minimap-toggle"
                    className="aria-pressed:bg-accent aria-pressed:text-foreground max-sm:hidden"
                    onClick={minimap.onToggle}
                >
                    <MapIcon aria-hidden="true" />
                </Button>
            ) : null}
        </div>
    )
}
