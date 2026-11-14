import { expect, it } from 'vitest'
import { diagnosticSentence, problemSentence } from '../problemText'

it('words validator problems as sentences, without the file the node already names', () => {
    expect(problemSentence('agent_task nodes need at least one route')).toBe('Agent task nodes need at least one route.')
    expect(problemSentence("node 'switch' has no routes, but only exit nodes can end the flow"))
        .toBe("Node 'switch' has no routes, but only exit nodes can end the flow.")
    // A message led by a file or ID keeps its case.
    expect(problemSentence('gate.md: human_gate nodes need at least one route.')).toBe('gate.md: human gate nodes need at least one route.')
    expect(diagnosticSentence({ node_id: 'gate', message: 'gate.md: human_gate nodes need at least one route' }))
        .toBe('Human gate nodes need at least one route.')
    expect(diagnosticSentence({ node_id: null, message: 'flow must have at least one exit node' })).toBe('Flow must have at least one exit node.')
})
