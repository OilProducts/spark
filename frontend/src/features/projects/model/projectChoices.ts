import type { RegisteredProject } from '@/state/store-types'
import { normalizeProjectPath } from '@/lib/projectPaths'

import { formatProjectListLabel } from './projectsHomeState'

type Registry = Record<string, RegisteredProject>

/** A project's name: Home by its display name, others by their folder. */
export function projectLabel(registry: Registry, projectPath: string | null | undefined): string {
    if (!projectPath) return 'No project'
    return registry[projectPath]?.displayName ?? formatProjectListLabel(projectPath)
}

const lastAccessed = (project: RegisteredProject) => project.lastAccessedAt ?? ''

/** Home first, then the other projects by most recent use. */
export function orderProjects(registry: Registry): RegisteredProject[] {
    return Object.values(registry).sort((left, right) => (
        Number(Boolean(right.isDefault)) - Number(Boolean(left.isDefault))
        || lastAccessed(right).localeCompare(lastAccessed(left))
        || left.directoryPath.localeCompare(right.directoryPath)
    ))
}

/** The project used most recently, or Home when none has been. */
export function lastUsedProjectPath(registry: Registry): string | null {
    const projects = Object.values(registry)
    const used = projects
        .filter((project) => project.lastAccessedAt)
        .sort((left, right) => lastAccessed(right).localeCompare(lastAccessed(left)))[0]
    return (used ?? projects.find((project) => project.isDefault) ?? projects[0])?.directoryPath ?? null
}

/**
 * The project a new chat, mission or flow run starts in: the one it came from
 * (such as the chat you were in) when that is still registered, else the last used.
 */
export function defaultProjectChoice(registry: Registry, origin?: string | null): string | null {
    return origin && registry[origin] ? origin : lastUsedProjectPath(registry)
}

export type ChatGroup<Chat> = { project: RegisteredProject; chats: Chat[] }

/** Each registered project with its chats, newest first, in project order. */
export function groupChatsByProject<Chat extends { updated_at: string }>(
    registry: Registry,
    chatsByProjectPath: Record<string, Chat[]>,
): ChatGroup<Chat>[] {
    return orderProjects(registry).map((project) => ({
        project,
        chats: [...(chatsByProjectPath[project.directoryPath] ?? [])]
            .sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
    }))
}

export const isFolderMissing = (project: RegisteredProject | undefined) => project?.folderExists === false

/** True when a trigger targets a path that is not a registered project. */
export function isUnregisteredTarget(registry: Registry, projectPath: string | null | undefined): boolean {
    if (!projectPath) return false
    return !registry[normalizeProjectPath(projectPath)]
}
