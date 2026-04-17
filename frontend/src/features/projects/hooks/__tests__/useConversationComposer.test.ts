import { parseConversationComposerCommand } from '@/features/projects/hooks/useConversationComposer'

describe('parseConversationComposerCommand', () => {
    it('parses bare mode switches', () => {
        expect(parseConversationComposerCommand('/plan')).toEqual({
            kind: 'switch_mode',
            chatMode: 'plan',
        })
        expect(parseConversationComposerCommand('/chat')).toEqual({
            kind: 'switch_mode',
            chatMode: 'chat',
        })
    })

    it('parses switch-and-send commands', () => {
        expect(parseConversationComposerCommand('/plan Draft the implementation plan.')).toEqual({
            kind: 'switch_and_send',
            chatMode: 'plan',
            message: 'Draft the implementation plan.',
        })
        expect(parseConversationComposerCommand('/chat Send the concise answer.')).toEqual({
            kind: 'switch_and_send',
            chatMode: 'chat',
            message: 'Send the concise answer.',
        })
    })

    it('leaves unknown slash commands alone', () => {
        expect(parseConversationComposerCommand('/unknown something')).toBeNull()
        expect(parseConversationComposerCommand('/planner')).toBeNull()
    })
})
