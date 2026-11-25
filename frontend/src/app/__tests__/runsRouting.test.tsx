import { selectSelectedRunId } from '@/state/runsSessionSelectors'
import { act, cleanup, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { HistoryRoutingController } from '@/app/AppSessionControllers'
import { EnvironmentBootstrap } from '@/app/EnvironmentSwitcher'
import { useActiveEnvironment } from '@/lib/activeEnvironment'
import { addsEntry, buildRunsHash, missionKey, parsePlaceHash, placeHash, roleKey, type Place } from '@/app/runsRouting'
// eslint-disable-next-line @typescript-eslint/no-restricted-imports -- triggerKey names a trigger; it calls no API
import { triggerKey, type TriggerResponse } from '@/lib/workspaceClient'
import { toHydratedProjectRecord } from '@/features/projects/model/projectsHomeState'
import { ROUTE_STATE_STORAGE_KEY, VIEW_MODE_SESSION_KEY } from '@/state/store-helpers'
import { resetStoreForEnvironmentSwitch, useStore } from '@/store'
import { switchEnvironment } from '@/test/environments'

// A fresh entry, so no Forward entries are left from an earlier test.
const resetHash = () => {
    window.history.pushState(null, '', window.location.pathname + window.location.search)
}

describe('runsRouting helpers', () => {
    it('round-trips run and node ids through the hash, encoding special characters', () => {
        expect(parsePlaceHash(buildRunsHash('run-123'))).toEqual({ view: 'runs', runId: 'run-123', nodeId: null })
        expect(parsePlaceHash(buildRunsHash('run-123', 'build-image'))).toEqual({
            view: 'runs',
            runId: 'run-123',
            nodeId: 'build-image',
        })
        expect(parsePlaceHash(buildRunsHash('run/slash', 'node space'))).toEqual({
            view: 'runs',
            runId: 'run/slash',
            nodeId: 'node space',
        })
    })

    it('names views by their labels and round-trips the open flow and trigger', () => {
        expect(placeHash({ view: 'editor', projectPath: null, flow: null })).toBe('#/flows')
        expect(placeHash({ view: 'home', projectPagePath: null })).toBe('#/chats')
        expect(placeHash({ view: 'projects' })).toBe('#/chats')
        expect(parsePlaceHash('#/flows')).toEqual({ view: 'editor' })
        expect(parsePlaceHash('#/chats')).toEqual({ view: 'home' })
        // Older links still open their view.
        expect(parsePlaceHash('#/editor')).toEqual({ view: 'editor' })
        expect(parsePlaceHash('#/home')).toEqual({ view: 'home' })

        const places: Place[] = [
            { view: 'editor', projectPath: '/tmp/a b', flow: 'demos/fix bug' },
            { view: 'editor', projectPath: '/tmp/a', flow: null },
            { view: 'triggers', triggerId: triggerKey({ project_path: '/tmp/a', id: 'nightly' }) },
            { view: 'missions', missionId: missionKey({ projectPath: '/tmp/a', id: 'mission-1' }) },
            { view: 'roles', roleId: roleKey({ projectPath: '/tmp/a', name: 'reviewer' }) },
        ]
        for (const place of places) expect(parsePlaceHash(placeHash(place))).toEqual(place)
        expect(placeHash(places[2])).toBe('#/triggers/nightly?project=%2Ftmp%2Fa')
        expect(placeHash(places[3])).toBe('#/missions/mission-1?project=%2Ftmp%2Fa')
        expect(placeHash({ view: 'missions', missionId: null })).toBe('#/missions')
        expect(placeHash(places[4])).toBe('#/roles/reviewer?project=%2Ftmp%2Fa')
        // Without a project, a flow link opens the flow in the project Flows shows.
        expect(parsePlaceHash('#/flows/fix-bug')).toEqual({ view: 'editor', flow: 'fix-bug' })
        expect(parsePlaceHash('#/flows/fix-bug', '/tmp/a')).toEqual({ view: 'editor', flow: 'fix-bug' })
        // A role, trigger or mission link without one names its item in the project in view.
        expect(parsePlaceHash('#/roles/reviewer', '/tmp/a')).toEqual(places[4])
        expect(parsePlaceHash('#/triggers/nightly', '/tmp/a')).toEqual(places[2])
        expect(parsePlaceHash('#/roles/reviewer?project=%2Ftmp%2Fb', '/tmp/a')).toEqual({ view: 'roles', roleId: roleKey({ projectPath: '/tmp/b', name: 'reviewer' }) })
    })

    it('adds no entry for the run a desktop opens by itself, with none open or in a missing one\'s place', () => {
        const store = useStore.getState()
        const state = { ...store, runsListSession: { ...store.runsListSession, missingRunId: 'gone', autoOpenedRunId: 'next' } }
        const runs = (runId: string | null): Place => ({ view: 'runs', runId })
        expect(addsEntry(state, runs(null), runs('next'))).toBe(false)
        // The 404 and the run opened in its place can land in one step.
        expect(addsEntry(state, runs('gone'), runs('next'))).toBe(false)
        expect(addsEntry(state, runs('other'), runs('next'))).toBe(true)
        expect(addsEntry(state, runs(null), runs('picked'))).toBe(true)
    })

    it('names a remote environment in its places\' links, and this machine in none', () => {
        const places: Place[] = [
            { envId: 'env-r', view: 'runs', runId: 'run-1', nodeId: 'build' },
            { envId: 'env r', view: 'triggers', triggerId: triggerKey({ project_path: '/tmp/a', id: 'nightly' }) },
            { envId: 'env-r', view: 'overview' },
        ]
        for (const place of places) expect(parsePlaceHash(placeHash(place))).toEqual(place)
        expect(placeHash(places[1])).toBe('#/triggers/nightly?project=%2Ftmp%2Fa&env=env%20r')
        expect(placeHash({ envId: null, view: 'runs', runId: 'run-1' })).toBe(buildRunsHash('run-1'))
        expect(parsePlaceHash(buildRunsHash('run-1'))).not.toHaveProperty('envId')
    })

    it('names no place for unknown or malformed hashes', () => {
        expect(parsePlaceHash('')).toBeNull()
        expect(parsePlaceHash('#/')).toBeNull()
        expect(parsePlaceHash('#/bogus')).toBeNull()
        expect(parsePlaceHash('#/projects')).toBeNull()
        expect(parsePlaceHash('#/runs/%E0')).toBeNull()
    })
})

describe('HistoryRoutingController', () => {
    beforeEach(() => {
        resetHash()
        useStore.setState({ viewMode: 'home', projectPagePath: null, activeProjectPath: null, activeFlow: null, runDetailSessionsByRunId: {} })
        useStore.getState().setRunsSelectedRunId(null)
        useStore.getState().updateTriggersSession({ triggers: [], selectedTriggerId: null })
    })

    afterEach(() => {
        cleanup()
        resetHash()
        window.innerWidth = 1024
    })

    it('applies a deep link hash to the store on mount', async () => {
        window.history.replaceState(null, '', buildRunsHash('run-deep-link', 'deploy'))

        render(<HistoryRoutingController />)

        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('runs')
        })
        expect(selectSelectedRunId(useStore.getState())).toBe('run-deep-link')
        expect(
            useStore.getState().runDetailSessionsByRunId['run-deep-link']?.selectedNodeId,
        ).toBe('deploy')
    })

    it('applies hashchange navigation to the store', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            window.location.hash = buildRunsHash('run-live', 'verify')
            window.dispatchEvent(new HashChangeEvent('hashchange'))
        })

        await waitFor(() => {
            expect(selectSelectedRunId(useStore.getState())).toBe('run-live')
        })
        expect(useStore.getState().viewMode).toBe('runs')
        expect(
            useStore.getState().runDetailSessionsByRunId['run-live']?.selectedNodeId,
        ).toBe('verify')
    })

    it('opens a view named by a bare hash link', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            window.location.hash = '#/flows'
            window.dispatchEvent(new HashChangeEvent('hashchange'))
        })

        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('editor')
        })
        expect(window.location.hash).toBe('#/flows')
    })

    it('puts the URL back to where the app is when a link names no place', async () => {
        useStore.setState({ viewMode: 'overview' })
        window.history.replaceState(null, '', '#/bogus')
        render(<HistoryRoutingController />)
        expect(window.location.hash).toBe('#/overview')

        act(() => {
            window.location.hash = '#/nowhere'
            window.dispatchEvent(new HashChangeEvent('hashchange'))
        })

        await waitFor(() => {
            expect(window.location.hash).toBe('#/overview')
        })
        expect(useStore.getState().viewMode).toBe('overview')
    })

    it('reflects store selection into the hash while on the runs tab', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            const state = useStore.getState()
            state.setViewMode('runs')
            state.setRunsSelectedRunId('run-store')
            state.updateRunDetailSession('run-store', { selectedNodeId: 'lint' })
        })

        await waitFor(() => {
            expect(window.location.hash).toBe(buildRunsHash('run-store', 'lint'))
        })
    })

    it('names the view in the hash after leaving the runs tab', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            const state = useStore.getState()
            state.setViewMode('runs')
            state.setRunsSelectedRunId('run-leaving')
        })
        await waitFor(() => {
            expect(window.location.hash).toBe(buildRunsHash('run-leaving'))
        })

        act(() => {
            useStore.getState().setViewMode('home')
        })

        await waitFor(() => {
            expect(window.location.hash).toBe('#/chats')
        })
    })

    it('adds one history entry per place, so Back returns to the view a run was opened from', async () => {
        useStore.setState({ viewMode: 'overview' })
        render(<HistoryRoutingController />)
        const start = window.history.length

        // The view and its run change in separate updates, as when a run opens from the Overview.
        await act(async () => {
            useStore.getState().setRunsSelectedRunId('run-opened')
            useStore.getState().setViewMode('runs')
        })
        expect(window.history.length).toBe(start + 1)
        // Picking a node rewrites the entry rather than adding one.
        await act(async () => {
            useStore.getState().updateRunDetailSession('run-opened', { selectedNodeId: 'build' })
        })
        expect(window.history.length).toBe(start + 1)
        expect(window.location.hash).toBe(buildRunsHash('run-opened', 'build'))

        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('overview')
        })
        expect(window.location.hash).toBe('#/overview')

        act(() => {
            window.history.forward()
        })
        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('runs')
        })
        expect(selectSelectedRunId(useStore.getState())).toBe('run-opened')
        expect(useStore.getState().runDetailSessionsByRunId['run-opened']?.selectedNodeId).toBe('build')
    })

    it('rewrites the entry of a linked run that isn\'t there, so Back doesn\'t reopen it', async () => {
        window.history.replaceState(null, '', buildRunsHash('run-gone'))
        render(<HistoryRoutingController />)
        await waitFor(() => expect(selectSelectedRunId(useStore.getState())).toBe('run-gone'))
        const start = window.history.length
        // The server says there is no such run.
        await act(async () => {
            useStore.getState().clearRunDetailSession('run-gone')
        })
        expect(useStore.getState().runsListSession.missingRunId).toBe('run-gone')
        expect(window.location.hash).toBe('#/runs')
        expect(window.history.length).toBe(start)
        // Opening another run puts the notice away.
        await act(async () => {
            useStore.getState().setRunsSelectedRunId('run-here')
        })
        expect(useStore.getState().runsListSession.missingRunId).toBeNull()
    })

    it('keeps the URL true when a confirmation holds a move', async () => {
        useStore.setState({ viewMode: 'overview' })
        render(<HistoryRoutingController />)
        await act(async () => {
            useStore.getState().setViewMode('settings')
        })
        // Unsaved settings hold every move away until it is confirmed.
        const hold = (event: Event) => event.preventDefault()
        window.addEventListener('spark:before-navigation', hold)
        try {
            // Back is held: its entry is put back.
            act(() => {
                window.history.back()
            })
            await waitFor(() => {
                expect(window.location.hash).toBe('#/settings')
            })
            expect(useStore.getState().viewMode).toBe('settings')

            // A held link's own entry says where the app still is.
            const length = window.history.length
            act(() => {
                window.location.hash = '#/flows'
                window.dispatchEvent(new HashChangeEvent('hashchange'))
            })
            await waitFor(() => {
                expect(window.location.hash).toBe('#/settings')
            })
            expect(window.history.length).toBe(length + 1)
            expect(useStore.getState().viewMode).toBe('settings')
        } finally {
            window.removeEventListener('spark:before-navigation', hold)
        }
    })

    it('adds an entry per flow opened in Flows, so Back reopens the previous flow', async () => {
        useStore.setState({ viewMode: 'editor', activeProjectPath: '/tmp/project-a' })
        render(<HistoryRoutingController />)
        const start = window.history.length

        await act(async () => {
            useStore.getState().setActiveFlow('fix-bug')
        })
        await act(async () => {
            useStore.getState().setActiveFlow('release-check')
        })
        expect(window.history.length).toBe(start + 2)
        expect(window.location.hash).toBe('#/flows/release-check?project=%2Ftmp%2Fproject-a')

        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState().activeFlow).toBe('fix-bug')
        })
        expect(useStore.getState().viewMode).toBe('editor')
        expect(window.location.hash).toBe('#/flows/fix-bug?project=%2Ftmp%2Fproject-a')
    })

    it('reopens a linked flow in its project', async () => {
        useStore.setState({ activeProjectPath: '/tmp/project-a', activeFlow: 'release-check' })
        window.history.replaceState(null, '', placeHash({ view: 'editor', projectPath: '/tmp/project-b', flow: 'demos/fix-bug' }))

        render(<HistoryRoutingController />)

        expect(useStore.getState().viewMode).toBe('editor')
        expect(useStore.getState().activeProjectPath).toBe('/tmp/project-b')
        expect(useStore.getState().activeFlow).toBe('demos/fix-bug')
        expect(window.location.hash).toBe('#/flows/demos%2Ffix-bug?project=%2Ftmp%2Fproject-b')
    })

    it('reopens a linked role, and Back from a flow it opened returns to it', async () => {
        useStore.setState({ selectedRole: null, viewMode: 'overview' })
        window.history.replaceState(null, '', '#/roles/reviewer?project=%2Ftmp%2Fproject-a')
        render(<HistoryRoutingController />)
        expect(useStore.getState()).toMatchObject({ viewMode: 'roles', selectedRole: { projectPath: '/tmp/project-a', name: 'reviewer' } })

        await act(async () => {
            useStore.getState().setSelectedRole({ projectPath: '/tmp/project-a', name: 'author' })
        })
        await act(async () => {
            useStore.getState().setViewMode('editor')
        })
        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState()).toMatchObject({ viewMode: 'roles', selectedRole: { projectPath: '/tmp/project-a', name: 'author' } })
        })
        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState().selectedRole).toEqual({ projectPath: '/tmp/project-a', name: 'reviewer' })
        })
    })

    it('opens a role linked without a project in the project in view, once the projects have loaded', async () => {
        useStore.setState({ selectedRole: null, viewMode: 'overview', projectRegistry: {} })
        window.history.replaceState(null, '', '#/roles/reviewer')
        render(<HistoryRoutingController />)
        expect(useStore.getState()).toMatchObject({ viewMode: 'roles', selectedRole: null })

        act(() => {
            useStore.setState({ projectRegistry: { '/tmp/project-a': { directoryPath: '/tmp/project-a', isFavorite: false, lastAccessedAt: null } } })
        })
        expect(useStore.getState().selectedRole).toEqual({ projectPath: '/tmp/project-a', name: 'reviewer' })
        await waitFor(() => expect(window.location.hash).toBe('#/roles/reviewer?project=%2Ftmp%2Fproject-a'))
    })

    it('reopens a linked mission, and Back returns to the list', async () => {
        useStore.setState({ selectedMission: null, viewMode: 'overview' })
        window.history.replaceState(null, '', '#/missions/mission-1?project=%2Ftmp%2Fproject-a')
        render(<HistoryRoutingController />)
        expect(useStore.getState().viewMode).toBe('missions')
        expect(useStore.getState().selectedMission).toEqual({ projectPath: '/tmp/project-a', id: 'mission-1' })

        await act(async () => {
            useStore.getState().setSelectedMission({ projectPath: '/tmp/project-a', id: 'mission-2' })
        })
        expect(window.location.hash).toBe('#/missions/mission-2?project=%2Ftmp%2Fproject-a')
        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState().selectedMission).toEqual({ projectPath: '/tmp/project-a', id: 'mission-1' })
        })
    })

    it('adds an entry per trigger picked, but not when Triggers picks one itself', async () => {
        const trigger = (id: string) => ({ project_path: '/tmp/project-a', id }) as TriggerResponse
        const [nightly, weekly] = [triggerKey(trigger('nightly')), triggerKey(trigger('weekly'))]
        window.innerWidth = 1440
        useStore.setState({ viewMode: 'triggers' })
        render(<HistoryRoutingController />)
        const start = window.history.length

        // With none picked yet, a desktop list picks its first trigger.
        await act(async () => {
            useStore.getState().updateTriggersSession({ triggers: [trigger('nightly'), trigger('weekly')], selectedTriggerId: nightly })
        })
        expect(window.history.length).toBe(start)
        expect(window.location.hash).toBe(placeHash({ view: 'triggers', triggerId: nightly }))

        await act(async () => {
            useStore.getState().updateTriggersSession({ selectedTriggerId: weekly })
        })
        expect(window.history.length).toBe(start + 1)

        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState().triggersSession.selectedTriggerId).toBe(nightly)
        })

        // A trigger that goes away is replaced in place, so Back never returns to it.
        act(() => {
            window.history.forward()
        })
        await waitFor(() => {
            expect(useStore.getState().triggersSession.selectedTriggerId).toBe(weekly)
        })
        await act(async () => {
            useStore.getState().updateTriggersSession({ triggers: [trigger('nightly')], selectedTriggerId: nightly })
        })
        expect(window.history.length).toBe(start + 1)
        expect(window.location.hash).toBe(placeHash({ view: 'triggers', triggerId: nightly }))
    })

    it('on a phone, adds an entry for a trigger picked from the list, so Back returns to the list', async () => {
        const nightly = triggerKey({ project_path: '/tmp/project-a', id: 'nightly' })
        window.innerWidth = 390
        useStore.setState({ viewMode: 'triggers' })
        useStore.getState().updateTriggersSession({ triggers: [{ project_path: '/tmp/project-a', id: 'nightly' } as TriggerResponse] })
        render(<HistoryRoutingController />)
        const start = window.history.length

        await act(async () => {
            useStore.getState().updateTriggersSession({ selectedTriggerId: nightly })
        })
        expect(window.history.length).toBe(start + 1)

        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState().triggersSession.selectedTriggerId).toBeNull()
        })
        expect(window.location.hash).toBe('#/triggers')
    })
})

describe('routing across environments', () => {
    const savedRoute = () => JSON.parse(window.localStorage.getItem(ROUTE_STATE_STORAGE_KEY) ?? '{}')
    const selectRun = (runId: string) => act(() => {
        useStore.getState().setRunsSelectedRunId(runId)
        useStore.getState().setViewMode('runs')
    })
    // As the app does: the switch resets the store while the workspace is still mounted, which then
    // remounts in the environment switched to.
    const Workspace = () => <HistoryRoutingController key={useActiveEnvironment().activeEnvironmentId ?? 'local'} />
    const showEnvironment = (environmentId: string | null) => act(() => switchEnvironment(environmentId))

    beforeEach(() => {
        resetHash()
        useStore.setState(useStore.getInitialState(), true)
        useStore.setState({ viewMode: 'home' })
        // This machine's own environment list, which the bootstrap loads, isn't needed here.
        vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 404 })))
    })

    afterEach(() => {
        cleanup()
        resetHash()
        vi.unstubAllGlobals()
    })

    it('keeps the entry a switch leaves for Back and Forward, leaving this machine where it is on another environment\'s entry', async () => {
        render(<><EnvironmentBootstrap /><Workspace /></>)
        selectRun('local-run')
        await waitFor(() => expect(window.location.hash).toBe(buildRunsHash('local-run')))
        const entries = window.history.length

        await showEnvironment('env-r')
        expect(window.history.length).toBe(entries + 1)
        // The local entry the switch happened on opens nothing remotely.
        expect(selectSelectedRunId(useStore.getState())).toBeNull()
        selectRun('remote-run')
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))

        await showEnvironment(null)
        expect(window.history.length).toBe(entries + 3)
        expect(useStore.getState().viewMode).toBe('runs')
        expect(selectSelectedRunId(useStore.getState())).toBe('local-run')
        expect(window.location.hash).toBe(buildRunsHash('local-run'))

        // Back reaches the remote run's entry: nothing here changes, nor the entry.
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        expect(window.history.state?.sparkPlace).toMatchObject({ envId: 'env-r', runId: 'remote-run' })
        expect(selectSelectedRunId(useStore.getState())).toBe('local-run')
        expect(savedRoute()).toMatchObject({ selectedRunId: 'local-run' })
        act(() => window.history.forward())
        await waitFor(() => expect(window.location.hash).toBe(buildRunsHash('local-run')))

        // Back past both switches reaches this machine's earlier entries.
        act(() => window.history.go(-4))
        await waitFor(() => expect(useStore.getState().viewMode).toBe('home'))
        expect(savedRoute()).toMatchObject({ selectedRunId: 'local-run' })
    })

    it('adds an entry of this machine\'s own for a project page opened here while a remote environment\'s entry shows', async () => {
        render(<><EnvironmentBootstrap /><Workspace /></>)
        await showEnvironment('env-r')
        selectRun('remote-run')
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        await showEnvironment(null)
        expect(useStore.getState().viewMode).toBe('home')
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))

        // The project page's link names no more than Chats', yet the move still leaves the remote entry.
        act(() => useStore.getState().openProjectPage('/tmp/project'))
        await waitFor(() => expect(window.location.hash).toBe('#/chats'))
        expect(window.history.state?.sparkPlace).toMatchObject({ envId: null, view: 'home', projectPagePath: '/tmp/project' })
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        expect(window.history.state?.sparkPlace).toMatchObject({ envId: 'env-r', runId: 'remote-run' })
        expect(useStore.getState().projectPagePath).toBe('/tmp/project')
    })

    it('adds an entry of this machine\'s own for another chat chosen here while a remote environment\'s entry shows', async () => {
        render(<><EnvironmentBootstrap /><Workspace /></>)
        await showEnvironment('env-r')
        selectRun('remote-run')
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        await showEnvironment(null)
        act(() => {
            useStore.getState().updateProjectSessionState('/tmp/project', { conversationId: 'chat-1' })
            useStore.setState({ activeProjectPath: '/tmp/project' })
        })
        await waitFor(() => expect(window.location.hash).toBe('#/chats'))
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))

        // No link names the chat, yet choosing another still leaves the remote entry.
        act(() => useStore.getState().updateProjectSessionState('/tmp/project', { conversationId: 'chat-2' }))
        await waitFor(() => expect(window.location.hash).toBe('#/chats'))
        expect(window.history.state?.sparkPlace).toMatchObject({ envId: null, view: 'home' })
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        expect(window.history.state?.sparkPlace).toMatchObject({ envId: 'env-r', runId: 'remote-run' })
        expect(useStore.getState().projectSessionsByPath['/tmp/project']?.conversationId).toBe('chat-2')
    })

    it('adds a clean entry of this machine\'s own for a settings category chosen here while a remote environment\'s entry shows', async () => {
        render(<><EnvironmentBootstrap /><Workspace /></>)
        await showEnvironment('env-r')
        selectRun('remote-run')
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        await showEnvironment(null)
        act(() => useStore.getState().setViewMode('settings'))
        await waitFor(() => expect(window.location.hash).toBe('#/settings'))
        act(() => window.history.go(-2))
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))

        act(() => useStore.getState().setSettingsCategory('appearance'))
        await waitFor(() => expect(window.location.hash).toBe('#/settings'))
        expect(window.history.state).toEqual({ sparkPlace: { envId: null, view: 'settings' } })
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        expect(useStore.getState().settingsCategory).toBe('appearance')
    })

    it('leaves Back and Forward as they are when this machine\'s projects load, opening one and its chat, while a remote environment\'s entry shows', async () => {
        render(<><EnvironmentBootstrap /><Workspace /></>)
        await showEnvironment('env-r')
        selectRun('remote-run')
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))
        await showEnvironment(null)
        const entries = window.history.length
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(`${buildRunsHash('remote-run')}?env=env-r`))

        act(() => useStore.getState().hydrateProjectRegistry([
            toHydratedProjectRecord({ project_path: '/tmp/project', is_favorite: false, active_conversation_id: 'chat-1', last_accessed_at: '2026-09-01T00:00:00Z' }),
        ]))
        expect(useStore.getState().activeProjectPath).toBe('/tmp/project')
        expect(useStore.getState().projectSessionsByPath['/tmp/project']?.conversationId).toBe('chat-1')
        await new Promise((resolve) => setTimeout(resolve, 0))
        expect(window.history.length).toBe(entries)
        expect(window.history.state?.sparkPlace).toMatchObject({ envId: 'env-r', runId: 'remote-run' })
        act(() => window.history.forward())
        await waitFor(() => expect(window.location.hash).toBe('#/chats'))
        expect(window.history.state?.sparkPlace).toMatchObject({ envId: null, view: 'home' })
    })

    it('keeps a remote environment\'s entries for Back and Forward to reach once it shows again', async () => {
        render(<><EnvironmentBootstrap /><Workspace /></>)
        await showEnvironment('env-r')
        for (const view of ['runs', 'triggers', 'settings'] as const) {
            act(() => useStore.getState().setViewMode(view))
            await waitFor(() => expect(window.location.hash).toBe(`#/${view}?env=env-r`))
        }
        await showEnvironment(null)
        const local = { hash: window.location.hash, route: savedRoute(), view: window.sessionStorage.getItem(`${VIEW_MODE_SESSION_KEY}:local`) }

        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe('#/settings?env=env-r'))
        expect(useStore.getState().viewMode).toBe('home')
        act(() => window.history.forward())
        await waitFor(() => expect(window.location.hash).toBe(local.hash))
        expect(useStore.getState().viewMode).toBe('home')
        expect(savedRoute()).toEqual(local.route)
        expect(window.sessionStorage.getItem(`${VIEW_MODE_SESSION_KEY}:local`)).toBe(local.view)

        await showEnvironment('env-r')
        expect(useStore.getState().viewMode).toBe('settings')
        // This machine's entry, kept by the switch, moves nothing here; the remote entries before it do.
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe(local.hash))
        expect(useStore.getState().viewMode).toBe('settings')
        act(() => window.history.go(-2))
        await waitFor(() => expect(useStore.getState().viewMode).toBe('triggers'))
        expect(window.location.hash).toBe('#/triggers?env=env-r')
    })

    it('adds an entry after a remote environment\'s entry for a move made here while it shows', async () => {
        render(<><EnvironmentBootstrap /><Workspace /></>)
        selectRun('local-run')
        await waitFor(() => expect(window.location.hash).toBe(buildRunsHash('local-run')))
        await showEnvironment('env-r')
        for (const view of ['runs', 'triggers'] as const) {
            act(() => useStore.getState().setViewMode(view))
            await waitFor(() => expect(window.location.hash).toBe(`#/${view}?env=env-r`))
        }
        await showEnvironment(null)
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe('#/triggers?env=env-r'))

        // A node change, which only rewrites its own entry, leaves the remote one as it is.
        act(() => useStore.getState().updateRunDetailSession('local-run', { selectedNodeId: 'build' }))
        await waitFor(() => expect(window.location.hash).toBe(buildRunsHash('local-run', 'build')))
        act(() => window.history.back())
        await waitFor(() => expect(window.location.hash).toBe('#/triggers?env=env-r'))
        expect(selectSelectedRunId(useStore.getState())).toBe('local-run')
    })

    it('opens nothing from a remote environment\'s link reloaded on this machine', async () => {
        window.history.replaceState(null, '', `${buildRunsHash('remote-run')}?env=env-r`)
        render(<HistoryRoutingController />)

        expect(window.location.hash).toBe('#/chats')
        expect(useStore.getState().viewMode).toBe('home')
        expect(selectSelectedRunId(useStore.getState())).toBeNull()
        expect(savedRoute()).not.toMatchObject({ selectedRunId: 'remote-run' })
    })

    it('keeps each environment\'s view across switches', async () => {
        act(() => useStore.getState().setViewMode('runs'))
        await switchEnvironment('env-r')
        resetStoreForEnvironmentSwitch(false)
        expect(useStore.getState().viewMode).toBe('overview')
        act(() => useStore.getState().setViewMode('settings'))
        expect(window.sessionStorage.getItem(`${VIEW_MODE_SESSION_KEY}:local`)).toBe('runs')

        await switchEnvironment(null)
        resetStoreForEnvironmentSwitch(true)
        expect(useStore.getState().viewMode).toBe('runs')
        await switchEnvironment('env-r')
        resetStoreForEnvironmentSwitch(false)
        expect(useStore.getState().viewMode).toBe('settings')
    })
})
