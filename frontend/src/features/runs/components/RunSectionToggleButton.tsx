import { ChevronDown, ChevronUp } from 'lucide-react'

import { Button } from '@/ui'

interface RunSectionToggleButtonProps {
    collapsed: boolean
    onToggle: () => void
    testId: string
}

export function RunSectionToggleButton({
    collapsed,
    onToggle,
    testId,
}: RunSectionToggleButtonProps) {
    return (
        <Button
            type="button"
            data-testid={testId}
            onClick={onToggle}
            variant="outline"
            size="xs"
            aria-expanded={!collapsed}
            aria-label={collapsed ? 'Expand section' : 'Collapse section'}
            className="h-7 gap-1.5 text-[11px] text-muted-foreground hover:text-foreground"
        >
            {collapsed ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}
            {collapsed ? 'Expand' : 'Collapse'}
        </Button>
    )
}
