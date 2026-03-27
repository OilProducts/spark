import { useNarrowViewport } from '@/lib/useNarrowViewport'

import { ExecutionControls } from './ExecutionControls'
import { ExecutionSidebar } from './ExecutionSidebar'

export function ExecutionWorkspace() {
    const isNarrowViewport = useNarrowViewport()

    return (
        <section
            data-testid="execution-workspace"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className="flex flex-1 overflow-hidden"
        >
            <div className={`flex h-full w-full overflow-hidden ${isNarrowViewport ? 'flex-col' : 'flex-row'}`}>
                <ExecutionSidebar />
                <ExecutionControls />
            </div>
        </section>
    )
}
