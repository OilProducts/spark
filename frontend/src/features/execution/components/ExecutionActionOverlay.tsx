import { Play } from 'lucide-react'

import { Button } from '@/ui'

interface ExecutionActionOverlayProps {
    isNarrowViewport: boolean
    disabled: boolean
    disabledReason?: string
    onExecute: () => void
}

export function ExecutionActionOverlay({
    isNarrowViewport,
    disabled,
    disabledReason,
    onExecute,
}: ExecutionActionOverlayProps) {
    return (
        <div
            data-testid="execution-canvas-primary-action"
            className={`absolute z-20 ${isNarrowViewport ? 'top-2 right-2' : 'top-4 right-4'}`}
        >
            <Button
                data-testid="execute-button"
                onClick={onExecute}
                disabled={disabled}
                title={disabledReason}
                className="shadow-lg"
            >
                <Play className="h-4 w-4" />
                Execute
            </Button>
        </div>
    )
}
