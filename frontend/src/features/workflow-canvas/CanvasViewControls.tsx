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

// The shown steps as `id` and label pairs, one per line, so moving a step re-renders nothing. They
// come in the flow's order, as a reader follows it: from its first steps along their routes, any
// step no route reaches after them, and the exits, where routes end, last.
const stepListSelector = (state: ReactFlowState) => {
    const next = new Map<string, string[]>()
    for (const edge of state.edges) next.set(edge.source, [...(next.get(edge.source) ?? []), edge.target])
    const targets = new Set(state.edges.map((edge) => edge.target))
    const order = new Map<string, number>()
    const queue = state.nodes.filter((node) => !targets.has(node.id)).map((node) => node.id)
    for (const id of queue) {
        if (order.has(id)) continue
        order.set(id, order.size)
        queue.push(...(next.get(id) ?? []))
    }
    const rank = (node: ReactFlowState['nodes'][number]) => (
        (node.type === 'exitNode' ? state.nodes.length : 0) + (order.get(node.id) ?? state.nodes.length - 1)
    )
    return state.nodes
        .filter((node) => !node.hidden)
        .sort((left, right) => rank(left) - rank(right))
        .map((node) => `${node.id}\t${(typeof node.data?.label === 'string' && node.data.label.trim()) || node.id}`)
        .join('\n')
}

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
        // It glows a moment, so the eye finds it among its neighbours.
        Array.from(store.getState().domNode?.querySelectorAll('.react-flow__node') ?? [])
            .find((element) => element.getAttribute('data-id') === id)?.animate?.(
            [{ filter: 'drop-shadow(0 0 10px hsl(var(--primary)))' }, { filter: 'none' }],
            { duration: 1600, easing: 'ease-out' },
        )
    }

    return (
        // On a phone, pinching zooms and a minimap would cover the graph: Fit and Lock stay, so the toolbar keeps to one row.
        // A narrow editor toolbar drops the zoom buttons (the wheel still zooms) and the minimap; beside the inspector, Lock too.
        <div role="group" aria-label="Zoom" data-testid="canvas-view-controls" className={cn('flex shrink-0 items-center gap-0.5', className)}>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Zoom out" title="Zoom out" className="max-sm:hidden @max-2xl/canvas-toolbar:hidden" onClick={() => void zoomOut({ duration: 150 })}>
                <Minus aria-hidden="true" />
            </Button>
            <Button type="button" size="icon-sm" variant="ghost" aria-label="Zoom in" title="Zoom in" className="max-sm:hidden @max-2xl/canvas-toolbar:hidden" onClick={() => void zoomIn({ duration: 150 })}>
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
                    className="aria-pressed:bg-accent aria-pressed:text-foreground lg:@max-xl/canvas-toolbar:hidden"
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
                    className="aria-pressed:bg-accent aria-pressed:text-foreground max-sm:hidden @max-2xl/canvas-toolbar:hidden"
                    onClick={minimap.onToggle}
                >
                    <MapIcon aria-hidden="true" />
                </Button>
            ) : null}
        </div>
    )
}
