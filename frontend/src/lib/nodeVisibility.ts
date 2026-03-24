import { getShapeHandlerType } from './workflowNodeShape'

export type HandlerType =
    | 'start'
    | 'exit'
    | 'codergen'
    | 'wait.human'
    | 'conditional'
    | 'parallel'
    | 'parallel.fan_in'
    | 'tool'
    | 'stack.manager_loop'
    | 'unknown'

export function getHandlerType(shape?: string, typeOverride?: string): HandlerType {
    const trimmedType = (typeOverride || '').trim()
    if (trimmedType) {
        return trimmedType as HandlerType
    }
    return getShapeHandlerType(shape)
}

export function getNodeFieldVisibility(handlerType: HandlerType) {
    const isStartOrExit = handlerType === 'start' || handlerType === 'exit'
    const isHumanOrConditional = handlerType === 'wait.human' || handlerType === 'conditional'

    const showPrompt = handlerType === 'codergen' || handlerType === 'parallel.fan_in'
    const showToolCommand = handlerType === 'tool'
    const showParallelOptions = handlerType === 'parallel'
    const showManagerOptions = handlerType === 'stack.manager_loop'
    const showHumanDefaultChoice = handlerType === 'wait.human'
    const showLlmSettings = handlerType === 'codergen' || handlerType === 'parallel.fan_in'
    const showTypeOverride = true

    const showAdvanced = !(isStartOrExit || isHumanOrConditional)
    const showGeneralAdvanced = showAdvanced

    return {
        showPrompt,
        showToolCommand,
        showParallelOptions,
        showManagerOptions,
        showHumanDefaultChoice,
        showTypeOverride,
        showAdvanced,
        showGeneralAdvanced,
        showLlmSettings,
    }
}
