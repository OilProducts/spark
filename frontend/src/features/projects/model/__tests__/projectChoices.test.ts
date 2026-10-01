import { describe, expect, it } from 'vitest'

import type { RegisteredProject } from '@/state/store-types'

import {
    defaultProjectChoice,
    groupChatsByProject,
    isFolderMissing,
    isUnregisteredTarget,
    launchWorkingDirectory,
    lastUsedProjectPath,
    orderProjects,
    projectLabel,
} from '../projectChoices'

const project = (directoryPath: string, overrides: Partial<RegisteredProject> = {}): RegisteredProject => ({
    directoryPath,
    isFavorite: false,
    lastAccessedAt: null,
    ...overrides,
})

const registry = Object.fromEntries([
    project('/work/old', { lastAccessedAt: '2026-09-01T00:00:00Z' }),
    project('/home/me', { displayName: 'Home', isDefault: true, lastAccessedAt: '2026-08-01T00:00:00Z' }),
    project('/work/new', { lastAccessedAt: '2026-09-30T00:00:00Z' }),
    project('/work/never'),
].map((entry) => [entry.directoryPath, entry]))

const chat = (id: string, updatedAt: string) => ({ conversation_id: id, updated_at: updatedAt })

describe('project choices', () => {
    it('orders Home first, then projects by most recent use', () => {
        expect(orderProjects(registry).map((entry) => entry.directoryPath)).toEqual(['/home/me', '/work/new', '/work/old', '/work/never'])
    })

    it('groups chats by project, Home first, each newest first, keeping projects without chats', () => {
        const groups = groupChatsByProject(registry, {
            '/work/old': [chat('old-a', '2026-09-01T00:00:00Z'), chat('old-b', '2026-09-02T00:00:00Z')],
            '/home/me': [chat('home-a', '2026-07-01T00:00:00Z')],
            '/unregistered': [chat('stray', '2026-09-30T00:00:00Z')],
        })
        expect(groups.map((group) => [group.project.directoryPath, group.chats.map((entry) => entry.conversation_id)])).toEqual([
            ['/home/me', ['home-a']],
            ['/work/new', []],
            ['/work/old', ['old-b', 'old-a']],
            ['/work/never', []],
        ])
    })

    it('names Home by its display name and other projects by their folder', () => {
        expect(projectLabel(registry, '/home/me')).toBe('Home')
        expect(projectLabel(registry, '/work/new')).toBe('new')
        expect(projectLabel(registry, '/elsewhere/repo')).toBe('repo')
    })

    it('starts new work in the project it came from, else the last used, else Home', () => {
        // A flow run from a chat suggests that chat's project.
        expect(defaultProjectChoice(registry, '/work/old')).toBe('/work/old')
        // A new mission (or a run with no originating chat) suggests the last used project.
        expect(defaultProjectChoice(registry)).toBe('/work/new')
        expect(defaultProjectChoice(registry, '/removed/project')).toBe('/work/new')
        expect(lastUsedProjectPath({ '/home/me': registry['/home/me'], '/work/never': registry['/work/never'] })).toBe('/home/me')
        const unused = { '/work/never': registry['/work/never'], '/home/me': { ...registry['/home/me'], lastAccessedAt: null } }
        expect(defaultProjectChoice(unused)).toBe('/home/me')
        expect(defaultProjectChoice({})).toBeNull()
    })

    it('marks a project whose folder is missing', () => {
        expect(isFolderMissing(project('/gone', { folderExists: false }))).toBe(true)
        expect(isFolderMissing(project('/here'))).toBe(false)
        expect(isFolderMissing(undefined)).toBe(false)
    })

    it('marks a trigger target that is not a registered project', () => {
        expect(isUnregisteredTarget(registry, '/work/new')).toBe(false)
        expect(isUnregisteredTarget(registry, '/work/new/')).toBe(false)
        expect(isUnregisteredTarget(registry, '/somewhere/else')).toBe(true)
        expect(isUnregisteredTarget(registry, null)).toBe(false)
    })
})

describe('launchWorkingDirectory', () => {
    it('keeps an explicit editor working directory over the chosen project', () => {
        expect(launchWorkingDirectory('/tmp/explicit', '/work/old', '/work/new')).toBe('/tmp/explicit')
    })

    it('defaults to the chosen project when no working directory was set', () => {
        expect(launchWorkingDirectory('./test-app', null, '/work/new')).toBe('/work/new')
        expect(launchWorkingDirectory('  ', null, '/work/new')).toBe('/work/new')
        // The field defaults to the chat's project; another chosen project still wins.
        expect(launchWorkingDirectory('/work/old', '/work/old', '/work/new')).toBe('/work/new')
    })
})
