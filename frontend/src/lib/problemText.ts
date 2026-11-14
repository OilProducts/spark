const KIND_WORDS: Record<string, string> = { agent_task: 'agent task', human_gate: 'human gate', fan_in: 'fan-in' }

/**
 * A validator message as a sentence: node kinds in words, a capital first
 * letter and a final period. One that starts with a file name ("notify.md: …")
 * keeps it, and a node ID that leads keeps its case.
 */
export function problemSentence(text: string): string {
    const worded = text.trim().replace(/\b(agent_task|human_gate|fan_in)\b/g, (kind) => KIND_WORDS[kind] ?? kind)
    const capitalized = worded.replace(/^((?:\S+\.md: )?)([a-z]+\s)/, (_, file: string, word: string) => `${file}${word.charAt(0).toUpperCase()}${word.slice(1)}`)
    return /[.!?]$/.test(capitalized) ? capitalized : `${capitalized}.`
}

/** `message` with each node it names by ID ("node 'step_1'") named by its label instead. */
export const nameNodes = (message: string, nodeLabel: (nodeId: string) => string) =>
    message.replace(/\bnode '([^']+)'/g, (_, nodeId: string) => `“${nodeLabel(nodeId)}”`)

/**
 * A diagnostic's message as a sentence, without the `<node>.md: ` prefix when the node is named
 * beside it. Given `nodeLabel`, a node the message names by ID ("node 'step_1'") goes by its label.
 */
export function diagnosticSentence(diagnostic: { message: string; node_id?: string | null }, nodeLabel?: (nodeId: string) => string): string {
    const prefix = diagnostic.node_id ? `${diagnostic.node_id}.md: ` : ''
    const message = prefix && diagnostic.message.startsWith(prefix) ? diagnostic.message.slice(prefix.length) : diagnostic.message
    return problemSentence(nodeLabel ? nameNodes(message, nodeLabel) : message)
}
