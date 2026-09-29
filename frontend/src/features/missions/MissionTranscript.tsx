import { useMemo, useState } from 'react'
import { useStore } from '@/store'
import { buildRunsScopeKey } from '@/state/runsSessionScope'
import { Button } from '@/components/ui/button'
import { InlineError } from '@/components/app/inline-error'
import { ProjectConversationHistory } from '@/features/projects/components/ProjectConversationHistory'
import {
    formatConversationTimestamp,
    getConversationFlowLaunches,
    getConversationTimelineEntries,
    hydrateConversationRecordFromSnapshot,
} from '@/features/projects/model/projectsHomeState'
import type { Mission } from './MissionsPanel'
import { useMissionConversation } from './hooks/useMissionConversation'

export function openRun(runId: string) {
    const state = useStore.getState()
    state.setRunsSelectedRunIdForScope(buildRunsScopeKey(state.runsListSession.scopeMode, state.activeProjectPath), runId)
    state.setViewMode('runs')
}

/** Delivered run events read `Run <id> (<flow>, "<summary>") ...`. */
const RUN_EVENT = /^Run (\S+) \(/
const noop = () => {}

/** The mission's conversation, with each delivered run event linked to the Runs view. */
export function MissionTranscript({ mission, project }: { mission: Mission; project: string }) {
    const { conversationId, snapshot, loading, error } = useMissionConversation(mission, project)
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const [expandedThinking, setExpandedThinking] = useState<Record<string, boolean>>({})
    const record = useMemo(() => snapshot ? hydrateConversationRecordFromSnapshot(snapshot) : null, [snapshot])
    const entries = getConversationTimelineEntries(record)
    const launches = getConversationFlowLaunches(record)
    return <>{error && <InlineError>{error}</InlineError>}<ProjectConversationHistory
        activeConversationId={conversationId}
        activeProjectPath={project}
        isConversationHistoryLoading={loading}
        hasRenderableConversationHistory={entries.length > 0}
        activeConversationHistory={entries}
        activeFlowRunRequestsById={new Map()}
        activeFlowLaunchesById={new Map(launches.map(launch => [launch.id, launch]))}
        activeProposedPlansById={new Map()}
        latestFlowRunRequestId={null}
        latestFlowLaunchId={launches.at(-1)?.id ?? null}
        expandedToolCalls={expandedToolCalls}
        expandedThinkingEntries={expandedThinking}
        pendingFlowRunRequestId={null}
        pendingProposedPlanId={null}
        requestUserInputActionError={null}
        submittingRequestUserInputIds={{}}
        formatConversationTimestamp={formatConversationTimestamp}
        onSubmitRequestUserInput={noop}
        onToggleToolCallExpanded={id => setExpandedToolCalls(current => ({ ...current, [id]: !current[id] }))}
        onToggleThinkingEntryExpanded={id => setExpandedThinking(current => ({ ...current, [id]: !current[id] }))}
        onReviewFlowRunRequest={noop}
        onReviewProposedPlan={noop}
        onOpenFlowRun={({ run_id }) => { if (run_id) openRun(run_id) }}
        renderUserMessage={(entry, key) => {
            const lines = entry.content.split('\n\n')
            if (!lines.some(line => RUN_EVENT.test(line))) return null
            return <li key={key} className="flex justify-end">
                <div aria-label="Mission events" className="max-w-[85%] space-y-2 rounded-md border border-border px-3 py-2 text-sm">
                    {lines.map((line, index) => {
                        const runId = RUN_EVENT.exec(line)?.[1]
                        return <p key={index} className="whitespace-pre-wrap break-words">{line}
                            {runId && <Button type="button" variant="link" size="sm" className="ml-2 h-auto p-0 align-baseline" aria-label={`Open run ${runId}`} onClick={() => openRun(runId)}>Open run</Button>}
                        </p>
                    })}
                    <time dateTime={entry.timestamp} className="block text-xs text-muted-foreground">{formatConversationTimestamp(entry.timestamp)}</time>
                </div>
            </li>
        }}
    /></>
}
