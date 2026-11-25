import { expect, it } from 'vitest'
import { buildPipelineContinuePayload, buildPipelineStartPayload, resolveExecutionWorkingDirectory } from '../pipelineStartPayload'

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

it('resolves a // working directory in a POSIX project as a POSIX path, not a UNC share', () => {
    const project = { projectPath: '/tmp/project', model: null, workingDirectory: ' //usr/local/../../bin ' }
    expect(buildPipelineStartPayload({ ...project, flowSource: 'a.dot' })).toMatchObject({ project_path: '/tmp/project', working_directory: '/bin' })
    expect(buildPipelineContinuePayload(project, { startNodeId: 'task', flowSourceMode: 'snapshot' }))
        .toMatchObject({ project_path: '/tmp/project', working_directory: '/bin' })
    // A Windows project still reads // as a UNC share root.
    expect(buildPipelineStartPayload({ ...project, projectPath: 'C:\\repo', flowSource: 'a.dot', workingDirectory: '\\\\server\\share\\..\\x' }))
        .toMatchObject({ working_directory: '//server/share/x' })
})

it('joins a relative working directory to the project before normalizing it', () => {
    // On POSIX a backslash is part of a folder name.
    expect(resolveExecutionWorkingDirectory({ projectPath: '/tmp/project', workingDirectory: 'team\\repo' })).toBe('/tmp/project/team\\repo')
    // A leading .. climbs the project, down to a UNC share's root.
    expect(resolveExecutionWorkingDirectory({ projectPath: 'C:\\repo', workingDirectory: '..\\worktree' })).toBe('C:/worktree')
    expect(resolveExecutionWorkingDirectory({ projectPath: '\\\\fileserver\\share\\repo', workingDirectory: '..\\other' })).toBe('//fileserver/share/other')
    expect(resolveExecutionWorkingDirectory({ projectPath: '\\\\fileserver\\share\\repo', workingDirectory: '..\\..\\..\\other' })).toBe('//fileserver/share/other')
})
