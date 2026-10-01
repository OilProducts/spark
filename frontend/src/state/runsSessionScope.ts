export function unconfirmRunQuestions(session: import('./viewSessionTypes').RunDetailSessionState) {
    if (session.questionsStatus === 'idle' && session.resourceRequestIds.questions === -1) return session
    return { ...session, questionsStatus: 'idle' as const,
        resourceRequestIds: { ...session.resourceRequestIds, questions: -1 },
    }
}
