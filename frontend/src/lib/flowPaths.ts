export interface FlowTreeDirectoryNode {
    kind: 'directory'
    name: string
    path: string
    children: FlowTreeNode[]
}

export interface FlowTreeFileNode {
    kind: 'flow'
    name: string
    path: string
}

export type FlowTreeNode = FlowTreeDirectoryNode | FlowTreeFileNode

export function splitFlowPath(flowPath: string): string[] {
    return flowPath
        .split('/')
        .map((segment) => segment.trim())
        .filter((segment) => segment.length > 0)
}

export function encodeFlowPath(flowPath: string): string {
    return splitFlowPath(flowPath).map((segment) => encodeURIComponent(segment)).join('/')
}

type MutableDirectory = {
    name: string
    path: string
    directories: Map<string, MutableDirectory>
    flows: FlowTreeFileNode[]
}

function createMutableDirectory(name: string, path: string): MutableDirectory {
    return {
        name,
        path,
        directories: new Map(),
        flows: [],
    }
}

function sortByName<T extends { name: string }>(entries: T[]): T[] {
    return [...entries].sort((left, right) => left.name.localeCompare(right.name))
}

function toTreeNodes(directory: MutableDirectory): FlowTreeNode[] {
    const directoryNodes = sortByName(Array.from(directory.directories.values())).map<FlowTreeDirectoryNode>((child) => ({
        kind: 'directory',
        name: child.name,
        path: child.path,
        children: toTreeNodes(child),
    }))
    const flowNodes = sortByName(directory.flows).map<FlowTreeFileNode>((flow) => ({
        kind: 'flow',
        name: flow.name,
        path: flow.path,
    }))
    return [...directoryNodes, ...flowNodes]
}

export function buildFlowTree(flowPaths: string[]): FlowTreeNode[] {
    const root = createMutableDirectory('', '')
    const seenFlows = new Set<string>()

    for (const flowPath of flowPaths) {
        const parts = splitFlowPath(flowPath)
        if (parts.length === 0) {
            continue
        }
        const normalizedPath = parts.join('/')
        if (seenFlows.has(normalizedPath)) {
            continue
        }
        seenFlows.add(normalizedPath)

        let current = root
        for (let index = 0; index < parts.length - 1; index += 1) {
            const directoryName = parts[index]
            const directoryPath = parts.slice(0, index + 1).join('/')
            let child = current.directories.get(directoryName)
            if (!child) {
                child = createMutableDirectory(directoryName, directoryPath)
                current.directories.set(directoryName, child)
            }
            current = child
        }

        current.flows.push({
            kind: 'flow',
            name: parts[parts.length - 1],
            path: normalizedPath,
        })
    }

    return toTreeNodes(root)
}
