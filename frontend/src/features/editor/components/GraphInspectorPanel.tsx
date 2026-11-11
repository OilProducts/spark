import { GraphSettings } from '../GraphSettings'

export function GraphInspectorPanel() {
    return (
        <div data-testid="graph-inspector-panel" className="flex-1 overflow-y-auto p-4">
            <GraphSettings inline />
        </div>
    )
}
