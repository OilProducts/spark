export type HandlerType =
    | 'start'
    | 'exit'
    | 'codergen'
    | 'wait.human'
    | 'switch'
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
    switch: 'switch',
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
    const isHumanOrSwitch = handlerType === 'wait.human' || handlerType === 'switch'

    const showPrompt = handlerType === 'codergen'
    const showToolCommand = handlerType === 'tool'
    const showParallelOptions = handlerType === 'parallel'
    const showManagerOptions = handlerType === 'stack.manager_loop'
    const showLlmSettings = handlerType === 'codergen'
    const showAdvanced = !(isStartOrExit || isHumanOrSwitch)
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
