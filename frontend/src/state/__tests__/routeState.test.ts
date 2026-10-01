import { beforeEach, expect, it } from 'vitest'

import { DEFAULT_ROUTE_STATE, loadRouteState, ROUTE_STATE_STORAGE_KEY, saveRouteState } from '../store-helpers'

beforeEach(() => window.localStorage.clear())

it('remembers the view and the last selection in each view', () => {
    const route = {
        viewMode: 'runs' as const,
        activeProjectPath: '/work/app',
        projectPagePath: '/home/me',
        selectedRunId: 'run-1',
        selectedTriggerId: 'trigger-1',
        selectedMission: { id: 'mission-1', projectPath: '/work/app' },
        activeFlow: 'team/review.yaml',
        settingsCategory: 'execution',
    }
    saveRouteState(route)
    expect(loadRouteState()).toEqual(route)
})

it('drops values it cannot use and falls back to the defaults', () => {
    expect(loadRouteState()).toEqual(DEFAULT_ROUTE_STATE)
    window.localStorage.setItem(ROUTE_STATE_STORAGE_KEY, JSON.stringify({
        viewMode: 'nowhere', activeProjectPath: 'relative/path', projectPagePath: 7,
        selectedRunId: '', selectedMission: { id: 'mission-1' }, activeFlow: null,
    }))
    expect(loadRouteState()).toEqual(DEFAULT_ROUTE_STATE)
    window.localStorage.setItem(ROUTE_STATE_STORAGE_KEY, '{not json')
    expect(loadRouteState()).toEqual(DEFAULT_ROUTE_STATE)
})

it('saves each view\'s selection as it changes', async () => {
    const { useStore } = await import('@/store')
    useStore.getState().setRunsSelectedRunId('run-saved')
    useStore.getState().setSelectedMission({ id: 'mission-saved', projectPath: '/work/app' })
    useStore.getState().updateTriggersSession({ selectedTriggerId: 'trigger-saved' })
    useStore.getState().setActiveFlow('saved.yaml')
    useStore.getState().setSettingsCategory('system')
    expect(JSON.parse(window.localStorage.getItem(ROUTE_STATE_STORAGE_KEY) ?? '{}')).toMatchObject({
        selectedRunId: 'run-saved',
        selectedMission: { id: 'mission-saved', projectPath: '/work/app' },
        selectedTriggerId: 'trigger-saved',
        activeFlow: 'saved.yaml',
        settingsCategory: 'system',
    })
})
