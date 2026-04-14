import type { MouseEvent, ReactNode } from 'react'
import { FilePlus } from 'lucide-react'

import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { FlowTree } from '@/components/app/flow-tree'

interface FlowBrowserPanelProps {
    activeFlow: string | null
    flows: string[]
    onCreateFlow: () => void | Promise<void>
    onDeleteFlow: (event: MouseEvent, fileName: string) => void | Promise<void>
    onSelectFlow: (flow: string | null) => void
    onRefresh?: () => void | Promise<void>
    isRefreshing?: boolean
    refreshButtonTestId?: string
    className?: string
    footerContent?: ReactNode
}

export function FlowBrowserPanel({
    activeFlow,
    flows,
    onCreateFlow,
    onDeleteFlow,
    onSelectFlow,
    onRefresh,
    isRefreshing = false,
    refreshButtonTestId,
    className,
    footerContent = null,
}: FlowBrowserPanelProps) {
    return (
        <div
            data-testid="flow-browser-panel"
            className={cn('flex flex-col overflow-hidden', className)}
        >
            <div className="flex items-center justify-between px-5 py-2">
                <h2 className="text-sm font-semibold tracking-tight">Saved Flows</h2>
                <div className="flex items-center gap-2">
                    {onRefresh ? (
                        <Button
                            type="button"
                            variant="outline"
                            size="xs"
                            data-testid={refreshButtonTestId}
                            onClick={() => {
                                void onRefresh()
                            }}
                            disabled={isRefreshing}
                        >
                            {isRefreshing ? 'Refreshing…' : 'Refresh'}
                        </Button>
                    ) : null}
                    <Button
                        type="button"
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => {
                            void onCreateFlow()
                        }}
                        title="New Flow"
                    >
                        <FilePlus className="h-4 w-4" />
                        <span className="sr-only">Create flow</span>
                    </Button>
                </div>
            </div>
            <div className="flex-1 space-y-4 overflow-y-auto px-3 pb-4">
                <FlowTree
                    dataTestId="editor-flow-tree"
                    flows={flows}
                    selectedFlow={activeFlow}
                    onSelectFlow={onSelectFlow}
                    onDeleteFlow={onDeleteFlow}
                />
                {footerContent}
            </div>
        </div>
    )
}
