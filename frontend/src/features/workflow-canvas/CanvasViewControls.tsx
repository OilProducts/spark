import { Lock, LockOpen, Map as MapIcon, Maximize, Minus, Plus } from 'lucide-react'
import { useReactFlow, useStore, useStoreApi } from '@xyflow/react'

import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

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

/**
 * Zoom, fit, minimap and lock buttons for a graph, shown in its toolbar rather than over the
 * canvas, so they never cover a node. Must render inside the graph's ReactFlowProvider.
 */
export function CanvasViewControls({ minimap, lockable = false, className }: CanvasViewControlsProps) {
    const { zoomIn, zoomOut, fitView } = useReactFlow()
    const store = useStoreApi()
    const isInteractive = useStore(isInteractiveSelector)

    return (
        <div role="group" aria-label="Zoom" data-testid="canvas-view-controls" className={cn('flex shrink-0 items-center gap-0.5', className)}>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Zoom out" title="Zoom out" onClick={() => void zoomOut({ duration: 150 })}>
                <Minus aria-hidden="true" />
            </Button>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Zoom in" title="Zoom in" onClick={() => void zoomIn({ duration: 150 })}>
                <Plus aria-hidden="true" />
            </Button>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Fit whole graph" title="Fit whole graph" onClick={() => void fitView({ padding: 0.1, duration: 200 })}>
                <Maximize aria-hidden="true" />
            </Button>
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
                    className="aria-pressed:bg-accent aria-pressed:text-foreground"
                    onClick={minimap.onToggle}
                >
                    <MapIcon aria-hidden="true" />
                </Button>
            ) : null}
        </div>
    )
}
