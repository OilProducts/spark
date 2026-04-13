import type { ReactNode } from 'react'

import { Card, CardContent, CardHeader } from '@/components/ui/card'

import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunAdvancedSectionProps {
    collapsed: boolean
    onCollapsedChange: (collapsed: boolean) => void
    children: ReactNode
}

export function RunAdvancedSection({
    collapsed,
    onCollapsedChange,
    children,
}: RunAdvancedSectionProps) {
    return (
        <Card data-testid="run-advanced-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Advanced</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Graph, checkpoint, context, and artifacts remain available here when deeper evidence is needed.
                        </p>
                    </div>
                    <RunSectionToggleButton
                        collapsed={collapsed}
                        onToggle={() => onCollapsedChange(!collapsed)}
                        testId="run-advanced-toggle-button"
                    />
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-6 px-4">
                    {children}
                </CardContent>
            ) : null}
        </Card>
    )
}
