import { expect, it } from 'vitest'
import { diagnosticSentence, problemSentence } from '../problemText'

it('words validator problems as sentences, without the file the node already names', () => {
    expect(problemSentence('agent_task steps need at least one route')).toBe('Agent steps need at least one route.')
    expect(problemSentence("node 'switch' has no routes, but only exit nodes can end the flow"))
        .toBe("Node 'switch' has no routes, but only exit nodes can end the flow.")
    // A file that leads stays as it is; the sentence after it starts with a capital. An ID that leads keeps its case.
    expect(problemSentence('gate.md: human_gate steps need at least one route.')).toBe('gate.md: Human gate steps need at least one route.')
    expect(problemSentence('step_1 needs a prompt')).toBe('step_1 needs a prompt.')
    expect(diagnosticSentence({ node_id: 'gate', message: 'gate.md: human_gate steps need at least one route' }))
        .toBe('Human gate steps need at least one route.')
    expect(diagnosticSentence({ node_id: null, message: 'flow must have at least one exit node' })).toBe('Flow must have at least one exit node.')
})

it('names a node by its label when one is given', () => {
    const labels: Record<string, string> = { step_1: 'Run Tests' }
    const label = (nodeId: string) => labels[nodeId] ?? nodeId
    expect(diagnosticSentence({ node_id: 'step_1', message: "node 'step_1' is unreachable: no route leads to it from the start node" }, label))
        .toBe('“Run Tests” is unreachable: no route leads to it from the start node.')
    expect(diagnosticSentence({ node_id: 'a', message: "branch node 'a' has no path to join 'j'" }, label)).toBe("Branch “a” has no path to join 'j'.")
})
