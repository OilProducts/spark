import { expect, it } from 'vitest'
import { buildPipelineContinuePayload, buildPipelineStartPayload } from '../pipelineStartPayload'

it('keeps the UNC share prefix in the project and working directory of start and continue requests', () => {
    const project = { projectPath: '\\\\fileserver\\share\\repo', model: null }
    expect(buildPipelineStartPayload({ ...project, flowSource: 'a.dot', workingDirectory: 'worktree' })).toMatchObject({
        project_path: '//fileserver/share/repo',
        working_directory: '//fileserver/share/repo/worktree',
    })
    expect(buildPipelineStartPayload({ ...project, flowSource: 'a.dot', workingDirectory: '' })).toMatchObject({
        working_directory: '//fileserver/share/repo',
    })
    expect(buildPipelineContinuePayload(
        { ...project, workingDirectory: '\\\\fileserver\\share\\repo\\wt' },
        { startNodeId: 'task', flowSourceMode: 'snapshot' },
    )).toMatchObject({ project_path: '//fileserver/share/repo', working_directory: '//fileserver/share/repo/wt' })
})
