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

const NODE_KIND_HANDLER_TYPE: Record<string, HandlerType> = {
    start: 'start',
    exit: 'exit',
    agent_task: 'codergen',
    human_gate: 'wait.human',
    conditional: 'conditional',
    parallel: 'parallel',
    fan_in: 'parallel.fan_in',
    tool: 'tool',
    subflow: 'stack.manager_loop',
}

export function getHandlerType(kind?: string): HandlerType {
    return NODE_KIND_HANDLER_TYPE[kind || ''] ?? 'unknown'
}

export function getNodeFieldVisibility(handlerType: HandlerType) {
    const isStartOrExit = handlerType === 'start' || handlerType === 'exit'
    const isHumanOrConditional = handlerType === 'wait.human' || handlerType === 'conditional'

    const showPrompt = handlerType === 'codergen' || handlerType === 'parallel.fan_in'
    const showToolCommand = handlerType === 'tool'
    const showParallelOptions = handlerType === 'parallel'
    const showManagerOptions = handlerType === 'stack.manager_loop'
    const showLlmSettings = handlerType === 'codergen' || handlerType === 'parallel.fan_in'
    const showAdvanced = !(isStartOrExit || isHumanOrConditional)
    const showGeneralAdvanced = showAdvanced

    return {
        showPrompt,
        showToolCommand,
        showParallelOptions,
        showManagerOptions,
        showAdvanced,
        showGeneralAdvanced,
        showLlmSettings,
    }
}
