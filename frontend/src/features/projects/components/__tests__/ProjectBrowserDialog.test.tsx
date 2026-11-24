import { cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { ProjectBrowserDialog } from '../ProjectBrowserDialog'

afterEach(cleanup)

/** Clicks every breadcrumb in order and returns the paths the dialog asked to browse. */
function breadcrumbPaths(currentPath: string) {
    const onBrowse = vi.fn()
    render(
        <ProjectBrowserDialog
            open
            currentPath={currentPath}
            parentPath={null}
            roots={[]}
            entries={[]}
            errorMessage={null}
            isLoading={false}
            onBrowse={onBrowse}
            onOpenChange={() => {}}
            onSelectCurrentFolder={() => {}}
        />,
    )
    within(screen.getByTestId('project-browser-breadcrumbs')).getAllByRole('button').forEach((button) => fireEvent.click(button))
    return onBrowse.mock.calls.map(([path]) => path)
}

it('roots a drive path at its drive and keeps its own separator', () => {
    expect(breadcrumbPaths('C:\\Users\\chris\\repo')).toEqual(['C:\\', 'C:\\Users', 'C:\\Users\\chris', 'C:\\Users\\chris\\repo'])
    cleanup()
    expect(breadcrumbPaths('C:/Users/chris/repo')).toEqual(['C:/', 'C:/Users', 'C:/Users/chris', 'C:/Users/chris/repo'])
})

it('roots a UNC path at its share', () => {
    expect(breadcrumbPaths('\\\\fileserver\\share\\team\\repo'))
        .toEqual(['\\\\fileserver\\share', '\\\\fileserver\\share\\team', '\\\\fileserver\\share\\team\\repo'])
    cleanup()
    expect(breadcrumbPaths('//fileserver/share/team/repo'))
        .toEqual(['//fileserver/share', '//fileserver/share/team', '//fileserver/share/team/repo'])
})

it('keeps the POSIX Root breadcrumb', () => {
    expect(breadcrumbPaths('/Users/chris/repo')).toEqual(['/', '/Users', '/Users/chris', '/Users/chris/repo'])
    expect(within(screen.getByTestId('project-browser-breadcrumbs')).getAllByRole('button')[0]).toHaveTextContent('Root')
})
