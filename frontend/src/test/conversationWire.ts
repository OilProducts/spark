// Test fixtures for the conversation wire protocol: rows, pages, batches, acknowledgements and
// summary snapshots, built from the transcript-shaped fixtures tests already use. Revisions,
// positions and orders are decimal strings, as on the wire.

type Fixture = Record<string, unknown>

export const TEST_GENERATION = 'generation-test'

/** A fixture's revision: an explicit `revision`, else its `updated_at` in seconds. */
export function fixtureRevision(fixture: Fixture): string {
    if (typeof fixture.revision === 'number' || typeof fixture.revision === 'string') {
        return String(fixture.revision)
    }
    const parsed = typeof fixture.updated_at === 'string' ? Date.parse(fixture.updated_at) : NaN
    return Number.isFinite(parsed) ? String(Math.floor(parsed / 1000)) : '1'
}

/** The conversation row payload for a conversation fixture (old snapshot or summary fields). */
export function conversationPayload(fixture: Fixture): Fixture {
    const turns = Array.isArray(fixture.turns) ? fixture.turns as Fixture[] : []
    const launches = Array.isArray(fixture.flow_launches) ? fixture.flow_launches as Fixture[] : []
    const lastMessage = [...turns].reverse().find((turn) => turn.kind !== 'mode_change' && typeof turn.content === 'string' && turn.content.trim())
    return {
        handle: fixture.conversation_handle ?? null,
        project_path: fixture.project_path,
        title: fixture.title ?? 'New thread',
        created_at: fixture.created_at ?? '',
        updated_at: fixture.updated_at ?? fixture.created_at ?? '',
        preview: fixture.last_message_preview ?? lastMessage?.content ?? null,
        running: fixture.running ?? turns.some((turn) => turn.role === 'assistant' && (turn.status === 'pending' || turn.status === 'streaming')),
        needs_attention: false,
        launched_run_ids: fixture.launched_run_ids ?? launches.flatMap((launch) => launch.run_id ? [launch.run_id] : []),
        metadata: {
            conversation_id: fixture.conversation_id,
            conversation_handle: fixture.conversation_handle ?? null,
            project_path: fixture.project_path,
            chat_mode: fixture.chat_mode ?? 'chat',
            provider: fixture.provider ?? null,
            model: fixture.model ?? null,
            reasoning_effort: fixture.reasoning_effort ?? null,
            title: fixture.title ?? 'New thread',
            settings_revision: String(fixture.settings_revision ?? fixtureRevision(fixture)),
        },
        ...(fixture.settings ? { settings: fixture.settings } : {}),
        effective: {
            provider: fixture.provider ?? null,
            model: fixture.model ?? null,
            llm_profile: null,
            reasoning_effort: fixture.reasoning_effort ?? null,
        },
        ...(fixture.standing_role !== undefined ? { standing_role: fixture.standing_role } : {}),
        ...(fixture.placeholder ? { placeholder: true } : {}),
    }
}

export function conversationRow(fixture: Fixture, revision = fixtureRevision(fixture)): Fixture {
    return {
        kind: 'conversation',
        conversation_id: fixture.conversation_id,
        id: fixture.conversation_id,
        revision,
        deleted: false,
        payload: conversationPayload(fixture),
    }
}

/**
 * A turn's position: the argument, else the turn fixture's own `position`, else its timestamp in
 * milliseconds, so a page and a later live row for the same turn agree.
 */
export function turnRow(conversationId: string, turn: Fixture, revision: string, position?: string | number): Fixture {
    const parsed = Date.parse(String(turn.timestamp))
    position ??= turn.position as string | number | undefined
    return {
        kind: 'turn',
        conversation_id: conversationId,
        id: turn.id,
        position: String(position ?? (Number.isFinite(parsed) ? parsed : 0)),
        revision,
        deleted: false,
        payload: turn,
    }
}

export function segmentRow(conversationId: string, segment: Fixture, revision: string): Fixture {
    return {
        kind: 'segment',
        conversation_id: conversationId,
        id: segment.id,
        turn_id: segment.turn_id,
        order: String(segment.order ?? 0),
        revision,
        deleted: false,
        payload: segment,
    }
}

export function artifactRow(conversationId: string, collection: string, artifact: Fixture, revision: string, position: number): Fixture {
    return {
        kind: 'artifact',
        conversation_id: conversationId,
        id: artifact.id,
        collection,
        ...(typeof artifact.source_turn_id === 'string' ? { turn_id: artifact.source_turn_id } : {}),
        position: String(position),
        revision,
        deleted: false,
        payload: artifact,
    }
}

export function tombstoneRow(kind: string, conversationId: string, id: string, revision: string, extra: Fixture = {}): Fixture {
    return { kind, conversation_id: conversationId, id, revision, deleted: true, payload: null, ...extra }
}

/** Every row of a transcript-shaped conversation fixture, at one revision. */
export function conversationRows(fixture: Fixture, revision = fixtureRevision(fixture)): Fixture[] {
    const conversationId = String(fixture.conversation_id)
    const turns = Array.isArray(fixture.turns) ? fixture.turns as Fixture[] : []
    const segments = Array.isArray(fixture.segments) ? fixture.segments as Fixture[] : []
    const artifacts = (collection: string) => (Array.isArray(fixture[collection]) ? fixture[collection] as Fixture[] : [])
        .map((artifact, index) => artifactRow(conversationId, collection, artifact, revision, index + 1))
    return [
        conversationRow(fixture, revision),
        ...turns.map((turn) => turnRow(conversationId, turn, revision)),
        ...segments.map((segment) => segmentRow(conversationId, segment, revision)),
        ...artifacts('flow_run_requests'),
        ...artifacts('flow_launches'),
        ...artifacts('proposed_plans'),
    ]
}

export function conversationPage(fixture: Fixture, options: {
    generation?: string
    watermark?: string
    older_cursor?: string | null
    revision?: string
    pinned_turn_ids?: string[]
} = {}): Fixture {
    const revision = options.revision ?? fixtureRevision(fixture)
    const rows = conversationRows(fixture, revision)
    return {
        generation: options.generation ?? TEST_GENERATION,
        watermark: options.watermark ?? revision,
        conversation_id: fixture.conversation_id,
        rows,
        turn_ids: rows.filter((row) => row.kind === 'turn').map((row) => row.id),
        pinned_turn_ids: options.pinned_turn_ids ?? [],
        pinned_artifact_ids: [],
        older_cursor: options.older_cursor ?? null,
    }
}

export function conversationBatch(rows: Fixture[], options: { generation?: string; from_revision?: string; watermark?: string } = {}): Fixture {
    const revisions = rows.map((row) => BigInt(String(row.revision)))
    const highest = revisions.reduce((max, revision) => (revision > max ? revision : max), 0n)
    const lowest = revisions.reduce((min, revision) => (revision < min ? revision : min), highest)
    const watermark = options.watermark ?? String(highest)
    return {
        generation: options.generation ?? TEST_GENERATION,
        // Just below the lowest row, so the batch is a valid catch-up range.
        from_revision: options.from_revision ?? String(rows.length > 0 ? lowest - 1n : BigInt(watermark)),
        watermark,
        rows,
    }
}

/** A settings, review or answer response: the fixture's rows as the mutation's changes. */
export function conversationMutationAck(fixture: Fixture, rows = conversationRows(fixture)): Fixture {
    const changes = conversationBatch(rows, { from_revision: '0' })
    return { generation: TEST_GENERATION, revision: fixtureRevision(fixture), changes }
}

/** A send or retry response accepting `userTurnId`. */
export function conversationAcceptance(fixture: Fixture, userTurnId: string, options: {
    assistant_turn_id?: string | null
    status?: 'accepted' | 'queued' | 'delivery_pending'
    rows?: Fixture[]
} = {}): Fixture {
    return {
        ...conversationMutationAck(fixture, options.rows),
        user_turn_id: userTurnId,
        client_turn_id: userTurnId,
        assistant_turn_id: options.assistant_turn_id ?? null,
        status: options.status ?? 'accepted',
    }
}

/** A thread-list response; each summary fixture becomes a conversation row. */
export function summarySnapshot(summaries: Fixture[], options: { project_path?: string | null; watermark?: string; generation?: string } = {}): Fixture {
    const rows = summaries.map((summary) => conversationRow(summary))
    const highest = rows.reduce((max, row) => (BigInt(String(row.revision)) > max ? BigInt(String(row.revision)) : max), 0n)
    return {
        generation: options.generation ?? TEST_GENERATION,
        watermark: options.watermark ?? String(highest),
        project_path: options.project_path ?? null,
        rows,
    }
}

/** A live envelope for one conversation (`conversation.window|changes|reset`) or the summary feed. */
export function liveEnvelope(type: string, payload: Fixture, scope: { conversationId?: string; projectPath?: string | null } = {}): Fixture {
    return {
        type,
        project_path: scope.projectPath ?? null,
        resource: scope.conversationId ? { kind: 'conversation', id: scope.conversationId } : { kind: 'conversations', id: null },
        cursor: null,
        payload,
    }
}
