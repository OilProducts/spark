const KIND_WORDS: Record<string, string> = { agent_task: 'agent task', human_gate: 'human gate', fan_in: 'fan-in' }

/**
 * A validator message as a sentence: node kinds in words, a capital first
 * letter and a final period. One that starts with a file or node name
 * ("notify.md: …") keeps its case.
 */
export function problemSentence(text: string): string {
    const worded = text.trim().replace(/\b(agent_task|human_gate|fan_in)\b/g, (kind) => KIND_WORDS[kind] ?? kind)
    const capitalized = /^[a-z]+\s/.test(worded) ? `${worded.charAt(0).toUpperCase()}${worded.slice(1)}` : worded
    return /[.!?]$/.test(capitalized) ? capitalized : `${capitalized}.`
}

/** A diagnostic's message as a sentence, without the `<node>.md: ` prefix when the node is named beside it. */
export function diagnosticSentence(diagnostic: { message: string; node_id?: string | null }): string {
    const prefix = diagnostic.node_id ? `${diagnostic.node_id}.md: ` : ''
    return problemSentence(prefix && diagnostic.message.startsWith(prefix) ? diagnostic.message.slice(prefix.length) : diagnostic.message)
}
