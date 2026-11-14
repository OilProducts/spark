import type { SaveErrorKind, SaveState } from '@/store'

export interface SaveRemediation {
    message: string
    allowRetry: boolean
}

export function resolveSaveRemediation(
    saveState: SaveState,
    saveErrorKind: SaveErrorKind | null
): SaveRemediation | null {
    if (saveState !== 'error' && saveState !== 'conflict') {
        return null
    }

    if (saveErrorKind === 'parse_error') {
        return {
            message: 'Fix the flow file syntax in the Files view, then save again.',
            allowRetry: false,
        }
    }
    if (saveErrorKind === 'validation_error') {
        return {
            message: 'Fix the errors listed under Validation; the flow saves with your next change.',
            allowRetry: false,
        }
    }
    if (saveErrorKind === 'conflict') {
        return {
            message: 'Reload the flow to get the saved version, then make your edits again.',
            allowRetry: false,
        }
    }
    return {
        message: 'Retry the save. If it fails again, check that Spark is still running.',
        allowRetry: true,
    }
}
