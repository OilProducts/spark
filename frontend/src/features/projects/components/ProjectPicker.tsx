import { useRef, type ReactNode } from 'react'
import { DropdownMenu } from 'radix-ui'

import { useStore } from '@/store'

import { isFolderMissing, orderProjects, projectLabel } from '../model/projectChoices'

const menuItem = 'relative flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50'

/**
 * Asks which project something starts in. Projects are listed Home first,
 * then by recent use; a default, when given, leads the list.
 */
export function ProjectPicker({
    heading,
    defaultProjectPath = null,
    onPick,
    testId,
    children,
}: {
    heading: string
    defaultProjectPath?: string | null
    onPick: (projectPath: string) => void
    testId: string
    children: ReactNode
}) {
    const registry = useStore((state) => state.projectRegistry)
    // A pick acts once the menu has closed, so what it opens can take focus;
    // only a dismissal returns focus to the trigger.
    const picked = useRef<string | null>(null)
    const ordered = orderProjects(registry)
    const projects = defaultProjectPath && registry[defaultProjectPath]
        ? [registry[defaultProjectPath], ...ordered.filter((project) => project.directoryPath !== defaultProjectPath)]
        : ordered
    return (
        <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>{children}</DropdownMenu.Trigger>
            <DropdownMenu.Portal>
                <DropdownMenu.Content
                    data-testid={testId}
                    align="start"
                    sideOffset={4}
                    collisionPadding={8}
                    onCloseAutoFocus={(event) => {
                        const projectPath = picked.current
                        picked.current = null
                        if (projectPath === null) return
                        event.preventDefault()
                        onPick(projectPath)
                    }}
                    className="z-50 max-h-[var(--radix-dropdown-menu-content-available-height)] min-w-60 overflow-y-auto rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md"
                >
                    <DropdownMenu.Label className="px-2 pb-1 pt-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground">{heading}</DropdownMenu.Label>
                    {projects.length === 0 ? (
                        <p className="px-2 py-1.5 text-xs text-muted-foreground">No projects yet.</p>
                    ) : projects.map((project) => (
                        <DropdownMenu.Item
                            key={project.directoryPath}
                            data-testid="project-picker-item"
                            data-project-path={project.directoryPath}
                            className={menuItem}
                            disabled={isFolderMissing(project)}
                            title={project.directoryPath}
                            onSelect={() => { picked.current = project.directoryPath }}
                        >
                            <span className="min-w-0 flex-1 truncate">{projectLabel(registry, project.directoryPath)}</span>
                            <span className="shrink-0 text-xs text-muted-foreground">
                                {isFolderMissing(project)
                                    ? 'folder missing'
                                    : project.directoryPath === defaultProjectPath
                                        ? 'suggested'
                                        : project.isDefault ? 'default' : ''}
                            </span>
                        </DropdownMenu.Item>
                    ))}
                </DropdownMenu.Content>
            </DropdownMenu.Portal>
        </DropdownMenu.Root>
    )
}
