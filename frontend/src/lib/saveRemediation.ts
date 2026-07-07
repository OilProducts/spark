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
            message: 'Fix YAML syntax issues in Raw YAML mode, then save again.',
            allowRetry: false,
        }
    }
    if (saveErrorKind === 'validation_error') {
        return {
            message: 'Resolve highlighted validation errors, then save again.',
            allowRetry: false,
        }
    }
    if (saveErrorKind === 'conflict') {
        return {
            message: 'Reload the flow to sync backend changes, then re-apply edits.',
            allowRetry: false,
        }
    }
    return {
        message: 'Retry save now, and verify backend connectivity if this repeats.',
        allowRetry: true,
    }
}
