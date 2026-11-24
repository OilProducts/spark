import { expect, it } from 'vitest'
import { isAbsoluteProjectPath, normalizeProjectPath } from '../projectPaths'

it('keeps a UNC share as the root and never climbs above it', () => {
    expect(normalizeProjectPath('\\\\fileserver\\share\\repo\\')).toBe('//fileserver/share/repo')
    expect(normalizeProjectPath('\\\\fileserver\\share')).toBe('//fileserver/share')
    expect(normalizeProjectPath('//fileserver/share//team/./repo')).toBe('//fileserver/share/team/repo')
    expect(normalizeProjectPath('\\\\fileserver\\share\\team\\..\\..\\..')).toBe('//fileserver/share')
})

it('normalizes drive and POSIX paths as before', () => {
    expect(normalizeProjectPath('C:\\Users\\chris\\repo\\')).toBe('C:/Users/chris/repo')
    expect(normalizeProjectPath('C:\\Users\\..\\..')).toBe('C:/')
    expect(normalizeProjectPath(' /tmp/demo//project/./subdir/.. ')).toBe('/tmp/demo/project')
    expect(normalizeProjectPath('///tmp/demo')).toBe('/tmp/demo')
    // On POSIX a backslash is part of a folder name.
    expect(normalizeProjectPath('/\\Team/repo')).toBe('/\\Team/repo')
    expect(normalizeProjectPath('/tmp/team\\repo')).toBe('/tmp/team\\repo')
    expect(normalizeProjectPath('/..')).toBe('/')
    expect(normalizeProjectPath('relative/./path')).toBe('relative/path')
})

it('recognizes POSIX, drive and UNC paths as absolute', () => {
    for (const path of ['/', '/tmp/repo', 'C:/repo', 'C:\\repo', '//server/share', '\\\\server\\share\\repo']) {
        expect(isAbsoluteProjectPath(path)).toBe(true)
    }
    for (const path of ['', 'repo', 'C:repo', '\\repo', '~', '~/repo']) {
        expect(isAbsoluteProjectPath(path)).toBe(false)
    }
})
