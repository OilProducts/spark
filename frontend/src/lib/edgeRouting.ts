export type EdgeRoutePoint = {
    x: number
    y: number
}

export type EdgeRoute = EdgeRoutePoint[]

export type NodeRect = {
    x: number
    y: number
    width: number
    height: number
}

export type RouteSide = 'top' | 'right' | 'bottom' | 'left'

export type RoutedPort = {
    side: RouteSide
    slot: number
    slotCount: number
}

export type FixedNodeRouterNode = {
    id: string
    rect: NodeRect
}

export type FixedNodeRouterEdge = {
    id: string
    source: string
    target: string
    sourcePort: RoutedPort
    targetPort: RoutedPort
    previousRoute?: EdgeRoute | null
}

export type FixedNodeRouterRequest = {
    nodes: FixedNodeRouterNode[]
    edges: FixedNodeRouterEdge[]
}

export type FixedNodeRouterResult = {
    routes: Record<string, EdgeRoute>
}

const DEFAULT_SLOT_OFFSET = 0.5
const PORT_LEAD_DISTANCE = 24
const OBSTACLE_PADDING = 18
const EDGE_CORNER_RADIUS = 12
const BEND_PENALTY = 18
const PREVIOUS_ROUTE_PENALTY = 1
// What a route pays to cross an edge routed before it: a short detour beats a crossing.
const CROSSING_PENALTY = 160
const SEGMENT_SEPARATION = 5
// Well inside OBSTACLE_PADDING, so a spread segment never reaches a node.
const MAX_SEGMENT_SHIFT = 10
const EPSILON = 0.001

type ExpandedRect = NodeRect

type Axis = 'horizontal' | 'vertical'

type GraphNode = {
    id: string
    point: EdgeRoutePoint
}

type SearchState = {
    cost: number
    nodeId: string
    axis: Axis | null
}

function isFinitePoint(point: EdgeRoutePoint | null | undefined): point is EdgeRoutePoint {
    return Boolean(
        point
        && Number.isFinite(point.x)
        && Number.isFinite(point.y),
    )
}

function arePointsEqual(a: EdgeRoutePoint, b: EdgeRoutePoint): boolean {
    return a.x === b.x && a.y === b.y
}

function isCollinear(a: EdgeRoutePoint, b: EdgeRoutePoint, c: EdgeRoutePoint): boolean {
    return (a.x === b.x && b.x === c.x) || (a.y === b.y && b.y === c.y)
}

export function normalizeRoute(route: EdgeRoute | null | undefined): EdgeRoute {
    const deduped = (route ?? []).filter(isFinitePoint).reduce<EdgeRoute>((points, point) => {
        if (points.length === 0 || !arePointsEqual(points[points.length - 1], point)) {
            points.push(point)
        }
        return points
    }, [])

    if (deduped.length <= 2) {
        return deduped
    }

    const compacted: EdgeRoute = [deduped[0]]
    for (let index = 1; index < deduped.length - 1; index += 1) {
        const previous = compacted[compacted.length - 1]
        const current = deduped[index]
        const next = deduped[index + 1]
        if (!isCollinear(previous, current, next)) {
            compacted.push(current)
        }
    }
    compacted.push(deduped[deduped.length - 1])
    return compacted
}

function moveTowards(from: EdgeRoutePoint, to: EdgeRoutePoint, distance: number): EdgeRoutePoint {
    if (from.x === to.x) {
        return {
            x: from.x,
            y: from.y + Math.sign(to.y - from.y) * distance,
        }
    }

    return {
        x: from.x + Math.sign(to.x - from.x) * distance,
        y: from.y,
    }
}

function isOrthogonalTurn(previous: EdgeRoutePoint, current: EdgeRoutePoint, next: EdgeRoutePoint): boolean {
    const incomingVertical = previous.x === current.x && previous.y !== current.y
    const incomingHorizontal = previous.y === current.y && previous.x !== current.x
    const outgoingVertical = current.x === next.x && current.y !== next.y
    const outgoingHorizontal = current.y === next.y && current.x !== next.x

    return (incomingVertical && outgoingHorizontal) || (incomingHorizontal && outgoingVertical)
}

export function buildRoundedPolylinePath(route: EdgeRoute | null | undefined): string {
    const normalizedRoute = normalizeRoute(route)
    if (normalizedRoute.length === 0) {
        return ''
    }

    if (normalizedRoute.length === 1) {
        return `M ${normalizedRoute[0].x} ${normalizedRoute[0].y}`
    }

    let path = `M ${normalizedRoute[0].x} ${normalizedRoute[0].y}`
    let cursor = normalizedRoute[0]

    for (let index = 1; index < normalizedRoute.length; index += 1) {
        const current = normalizedRoute[index]
        const previous = normalizedRoute[index - 1]
        const next = normalizedRoute[index + 1]

        if (!next || !isOrthogonalTurn(previous, current, next)) {
            if (!arePointsEqual(cursor, current)) {
                path += ` L ${current.x} ${current.y}`
                cursor = current
            }
            continue
        }

        const incomingLength = Math.hypot(current.x - previous.x, current.y - previous.y)
        const outgoingLength = Math.hypot(next.x - current.x, next.y - current.y)
        const radius = Math.min(EDGE_CORNER_RADIUS, incomingLength / 2, outgoingLength / 2)

        if (radius <= 0) {
            if (!arePointsEqual(cursor, current)) {
                path += ` L ${current.x} ${current.y}`
                cursor = current
            }
            continue
        }

        const cornerEntry = moveTowards(current, previous, radius)
        const cornerExit = moveTowards(current, next, radius)

        if (!arePointsEqual(cursor, cornerEntry)) {
            path += ` L ${cornerEntry.x} ${cornerEntry.y}`
        }
        path += ` Q ${current.x} ${current.y} ${cornerExit.x} ${cornerExit.y}`
        cursor = cornerExit
    }

    return path
}

export function getRouteMidpoint(route: EdgeRoute | null | undefined): EdgeRoutePoint {
    const normalizedRoute = normalizeRoute(route)
    if (normalizedRoute.length === 0) {
        return { x: 0, y: 0 }
    }
    if (normalizedRoute.length === 1) {
        return normalizedRoute[0]
    }

    const segmentLengths = normalizedRoute.slice(1).map((point, index) => {
        const previous = normalizedRoute[index]
        return Math.hypot(point.x - previous.x, point.y - previous.y)
    })
    const totalLength = segmentLengths.reduce((sum, segmentLength) => sum + segmentLength, 0)
    if (totalLength === 0) {
        return normalizedRoute[0]
    }

    const midpointDistance = totalLength / 2
    let traversed = 0

    for (let index = 0; index < segmentLengths.length; index += 1) {
        const segmentLength = segmentLengths[index]
        const previous = normalizedRoute[index]
        const next = normalizedRoute[index + 1]
        if (traversed + segmentLength >= midpointDistance) {
            const segmentOffset = midpointDistance - traversed
            const ratio = segmentLength === 0 ? 0 : segmentOffset / segmentLength
            return {
                x: previous.x + (next.x - previous.x) * ratio,
                y: previous.y + (next.y - previous.y) * ratio,
            }
        }
        traversed += segmentLength
    }

    return normalizedRoute[normalizedRoute.length - 1]
}

export function getSlotOffset(slot: number | null | undefined, slotCount: number | null | undefined): number {
    if (!Number.isFinite(slot) || !Number.isFinite(slotCount) || (slotCount as number) <= 0) {
        return DEFAULT_SLOT_OFFSET
    }
    return Math.min(1, Math.max(0, ((slot as number) + 1) / ((slotCount as number) + 1)))
}

export function getPortAnchorPoint(rect: NodeRect, port: RoutedPort): EdgeRoutePoint {
    const offset = getSlotOffset(port.slot, port.slotCount)
    if (port.side === 'top') {
        return { x: rect.x + rect.width * offset, y: rect.y }
    }
    if (port.side === 'right') {
        return { x: rect.x + rect.width, y: rect.y + rect.height * offset }
    }
    if (port.side === 'bottom') {
        return { x: rect.x + rect.width * offset, y: rect.y + rect.height }
    }
    return { x: rect.x, y: rect.y + rect.height * offset }
}

function getPortLeadPoint(anchor: EdgeRoutePoint, side: RouteSide): EdgeRoutePoint {
    if (side === 'top') {
        return { x: anchor.x, y: anchor.y - PORT_LEAD_DISTANCE }
    }
    if (side === 'right') {
        return { x: anchor.x + PORT_LEAD_DISTANCE, y: anchor.y }
    }
    if (side === 'bottom') {
        return { x: anchor.x, y: anchor.y + PORT_LEAD_DISTANCE }
    }
    return { x: anchor.x - PORT_LEAD_DISTANCE, y: anchor.y }
}

// The lead steps out of the port, but never into another node's padding: in a gap narrower
// than the lead it stops halfway across the free lane, so it can never jump across a neighbour.
// Nodes too close for any lane leave the lead inside the neighbour, and the search falls back.
function getPortEscapePoint(
    anchor: EdgeRoutePoint,
    side: RouteSide,
    obstacles: ExpandedRect[],
): EdgeRoutePoint {
    const horizontal = side === 'left' || side === 'right'
    const direction = side === 'right' || side === 'bottom' ? 1 : -1
    const along = horizontal ? anchor.x : anchor.y
    const across = horizontal ? anchor.y : anchor.x
    let reach = PORT_LEAD_DISTANCE
    obstacles.forEach((obstacle) => {
        const acrossMin = horizontal ? obstacle.y : obstacle.x
        const acrossMax = acrossMin + (horizontal ? obstacle.height : obstacle.width)
        if (across <= acrossMin + EPSILON || across >= acrossMax - EPSILON) {
            return
        }
        const nearMin = horizontal ? obstacle.x : obstacle.y
        const near = direction > 0 ? nearMin : nearMin + (horizontal ? obstacle.width : obstacle.height)
        const gap = (near - along) * direction
        if (gap > EPSILON && gap < reach) {
            reach = Math.max(OBSTACLE_PADDING, (OBSTACLE_PADDING + gap) / 2)
        }
    })
    const offset = reach * direction
    return horizontal ? { x: anchor.x + offset, y: anchor.y } : { x: anchor.x, y: anchor.y + offset }
}

function expandRect(rect: NodeRect, padding: number): ExpandedRect {
    return {
        x: rect.x - padding,
        y: rect.y - padding,
        width: rect.width + padding * 2,
        height: rect.height + padding * 2,
    }
}

function isStrictlyInsideRect(point: EdgeRoutePoint, rect: NodeRect): boolean {
    return (
        point.x > rect.x + EPSILON
        && point.x < rect.x + rect.width - EPSILON
        && point.y > rect.y + EPSILON
        && point.y < rect.y + rect.height - EPSILON
    )
}

function hasOverlap(minA: number, maxA: number, minB: number, maxB: number): boolean {
    return minA < maxB - EPSILON && maxA > minB + EPSILON
}

function segmentCrossesRect(a: EdgeRoutePoint, b: EdgeRoutePoint, rect: NodeRect): boolean {
    if (a.x === b.x) {
        if (a.x <= rect.x + EPSILON || a.x >= rect.x + rect.width - EPSILON) {
            return false
        }
        const minY = Math.min(a.y, b.y)
        const maxY = Math.max(a.y, b.y)
        return hasOverlap(minY, maxY, rect.y, rect.y + rect.height)
    }

    if (a.y === b.y) {
        if (a.y <= rect.y + EPSILON || a.y >= rect.y + rect.height - EPSILON) {
            return false
        }
        const minX = Math.min(a.x, b.x)
        const maxX = Math.max(a.x, b.x)
        return hasOverlap(minX, maxX, rect.x, rect.x + rect.width)
    }

    return false
}

function segmentIsClear(a: EdgeRoutePoint, b: EdgeRoutePoint, obstacles: ExpandedRect[]): boolean {
    if ((a.x !== b.x && a.y !== b.y) || !isFinitePoint(a) || !isFinitePoint(b)) {
        return false
    }

    return obstacles.every((obstacle) => !segmentCrossesRect(a, b, obstacle))
}

export function routeIntersectsRect(
    route: EdgeRoute | null | undefined,
    rect: NodeRect,
    padding = 0,
): boolean {
    const normalizedRoute = normalizeRoute(route)
    if (normalizedRoute.length === 0) {
        return false
    }

    const expandedRect = padding > 0 ? expandRect(rect, padding) : rect
    if (normalizedRoute.some((point) => isStrictlyInsideRect(point, expandedRect))) {
        return true
    }

    for (let index = 1; index < normalizedRoute.length; index += 1) {
        if (segmentCrossesRect(normalizedRoute[index - 1], normalizedRoute[index], expandedRect)) {
            return true
        }
    }

    return false
}

function createPointId(point: EdgeRoutePoint): string {
    return `${point.x}:${point.y}`
}

function compareNumber(left: number, right: number): number {
    return left - right
}

function buildGraphNodes(
    xs: number[],
    ys: number[],
    obstacles: ExpandedRect[],
    requiredPoints: EdgeRoutePoint[],
): Map<string, GraphNode> {
    const graphNodes = new Map<string, GraphNode>()

    requiredPoints.forEach((point) => {
        graphNodes.set(createPointId(point), {
            id: createPointId(point),
            point,
        })
    })

    xs.forEach((x) => {
        ys.forEach((y) => {
            const point = { x, y }
            if (obstacles.some((obstacle) => isStrictlyInsideRect(point, obstacle))) {
                return
            }
            graphNodes.set(createPointId(point), {
                id: createPointId(point),
                point,
            })
        })
    })

    return graphNodes
}

type PreviousRouteSegment = {
    axis: Axis
    min: number
    max: number
    coordinate: number
}

function buildPreviousRouteSegments(route: EdgeRoute | null | undefined): PreviousRouteSegment[] {
    const normalizedRoute = normalizeRoute(route)
    const segments: PreviousRouteSegment[] = []

    for (let index = 1; index < normalizedRoute.length; index += 1) {
        const previous = normalizedRoute[index - 1]
        const current = normalizedRoute[index]
        if (previous.x === current.x) {
            segments.push({
                axis: 'vertical',
                coordinate: previous.x,
                min: Math.min(previous.y, current.y),
                max: Math.max(previous.y, current.y),
            })
            continue
        }
        if (previous.y === current.y) {
            segments.push({
                axis: 'horizontal',
                coordinate: previous.y,
                min: Math.min(previous.x, current.x),
                max: Math.max(previous.x, current.x),
            })
        }
    }

    return segments
}

function segmentMatchesPreviousRoute(
    from: EdgeRoutePoint,
    to: EdgeRoutePoint,
    previousSegments: PreviousRouteSegment[],
): boolean {
    if (from.x === to.x) {
        return previousSegments.some((segment) =>
            segment.axis === 'vertical'
            && segment.coordinate === from.x
            && hasOverlap(
                Math.min(from.y, to.y),
                Math.max(from.y, to.y),
                segment.min,
                segment.max,
            ),
        )
    }

    if (from.y === to.y) {
        return previousSegments.some((segment) =>
            segment.axis === 'horizontal'
            && segment.coordinate === from.y
            && hasOverlap(
                Math.min(from.x, to.x),
                Math.max(from.x, to.x),
                segment.min,
                segment.max,
            ),
        )
    }

    return false
}

function buildAdjacency(
    graphNodes: Map<string, GraphNode>,
    xs: number[],
    ys: number[],
    obstacles: ExpandedRect[],
): Map<string, GraphNode[]> {
    const adjacency = new Map<string, GraphNode[]>()
    const getPoint = (x: number, y: number) => graphNodes.get(createPointId({ x, y })) ?? null

    ys.forEach((y) => {
        let previousNode: GraphNode | null = null
        xs.forEach((x) => {
            const node = getPoint(x, y)
            if (!node) {
                previousNode = null
                return
            }
            if (previousNode && segmentIsClear(previousNode.point, node.point, obstacles)) {
                const previousNeighbors = adjacency.get(previousNode.id) ?? []
                previousNeighbors.push(node)
                adjacency.set(previousNode.id, previousNeighbors)

                const nodeNeighbors = adjacency.get(node.id) ?? []
                nodeNeighbors.push(previousNode)
                adjacency.set(node.id, nodeNeighbors)
            } else if (previousNode && !segmentIsClear(previousNode.point, node.point, obstacles)) {
                previousNode = null
            }

            if (!previousNode || segmentIsClear(previousNode.point, node.point, obstacles)) {
                previousNode = node
            }
        })
    })

    xs.forEach((x) => {
        let previousNode: GraphNode | null = null
        ys.forEach((y) => {
            const node = getPoint(x, y)
            if (!node) {
                previousNode = null
                return
            }
            if (previousNode && segmentIsClear(previousNode.point, node.point, obstacles)) {
                const previousNeighbors = adjacency.get(previousNode.id) ?? []
                previousNeighbors.push(node)
                adjacency.set(previousNode.id, previousNeighbors)

                const nodeNeighbors = adjacency.get(node.id) ?? []
                nodeNeighbors.push(previousNode)
                adjacency.set(node.id, nodeNeighbors)
            } else if (previousNode && !segmentIsClear(previousNode.point, node.point, obstacles)) {
                previousNode = null
            }

            if (!previousNode || segmentIsClear(previousNode.point, node.point, obstacles)) {
                previousNode = node
            }
        })
    })

    adjacency.forEach((neighbors, key) => {
        adjacency.set(
            key,
            neighbors.sort((left, right) => {
                if (left.point.x !== right.point.x) {
                    return compareNumber(left.point.x, right.point.x)
                }
                return compareNumber(left.point.y, right.point.y)
            }),
        )
    })

    return adjacency
}

// How many routed segments the segment a-b crosses at a right angle, away from their ends.
function countCrossings(a: EdgeRoutePoint, b: EdgeRoutePoint, routedSegments: PreviousRouteSegment[]): number {
    const axis: Axis = a.x === b.x ? 'vertical' : 'horizontal'
    const [coordinate, from, to] = axis === 'vertical' ? [a.x, a.y, b.y] : [a.y, a.x, b.x]
    const min = Math.min(from, to)
    const max = Math.max(from, to)
    let crossings = 0
    routedSegments.forEach((segment) => {
        if (
            segment.axis !== axis
            && segment.coordinate > min + EPSILON && segment.coordinate < max - EPSILON
            && coordinate > segment.min + EPSILON && coordinate < segment.max - EPSILON
        ) {
            crossings += 1
        }
    })
    return crossings
}

function buildSearchKey(nodeId: string, axis: Axis | null): string {
    return `${nodeId}|${axis ?? 'start'}`
}

function findShortestOrthogonalPath(
    graphNodes: Map<string, GraphNode>,
    adjacency: Map<string, GraphNode[]>,
    startNodeId: string,
    endNodeId: string,
    previousRoute: EdgeRoute | null | undefined,
    routedSegments: PreviousRouteSegment[],
): EdgeRoute | null {
    const previousSegments = buildPreviousRouteSegments(previousRoute)
    const distances = new Map<string, number>([[buildSearchKey(startNodeId, null), 0]])
    const previousByState = new Map<string, { nodeId: string; axis: Axis | null } | null>([
        [buildSearchKey(startNodeId, null), null],
    ])
    const queue: SearchState[] = [{
        cost: 0,
        nodeId: startNodeId,
        axis: null,
    }]

    while (queue.length > 0) {
        queue.sort((left, right) => left.cost - right.cost)
        const current = queue.shift()
        if (!current) {
            break
        }
        const currentKey = buildSearchKey(current.nodeId, current.axis)
        if (current.cost > (distances.get(currentKey) ?? Number.POSITIVE_INFINITY)) {
            continue
        }
        if (current.nodeId === endNodeId) {
            const points: EdgeRoutePoint[] = []
            let stateKey: string | null = currentKey
            while (stateKey) {
                const [nodeId, axisLabel] = stateKey.split('|')
                points.push(graphNodes.get(nodeId)?.point ?? { x: 0, y: 0 })
                const previousState: { nodeId: string; axis: Axis | null } | null =
                    previousByState.get(stateKey) ?? null
                stateKey = previousState ? buildSearchKey(previousState.nodeId, previousState.axis) : null
                if (axisLabel === 'start' && !previousState) {
                    break
                }
            }
            return normalizeRoute(points.reverse())
        }

        const currentNode = graphNodes.get(current.nodeId)
        if (!currentNode) {
            continue
        }

        const neighbors = adjacency.get(current.nodeId) ?? []
        neighbors.forEach((neighbor) => {
            const axis: Axis = currentNode.point.x === neighbor.point.x ? 'vertical' : 'horizontal'
            const segmentLength = Math.abs(currentNode.point.x - neighbor.point.x) + Math.abs(currentNode.point.y - neighbor.point.y)
            const bendPenalty = current.axis && current.axis !== axis ? BEND_PENALTY : 0
            const previousRoutePenalty = previousSegments.length > 0
                && !segmentMatchesPreviousRoute(currentNode.point, neighbor.point, previousSegments)
                ? PREVIOUS_ROUTE_PENALTY
                : 0
            const crossingPenalty = CROSSING_PENALTY * countCrossings(currentNode.point, neighbor.point, routedSegments)
            const nextCost = current.cost + segmentLength + bendPenalty + previousRoutePenalty + crossingPenalty
            const nextKey = buildSearchKey(neighbor.id, axis)
            if (nextCost >= (distances.get(nextKey) ?? Number.POSITIVE_INFINITY)) {
                return
            }

            distances.set(nextKey, nextCost)
            previousByState.set(nextKey, {
                nodeId: current.nodeId,
                axis: current.axis,
            })
            queue.push({
                cost: nextCost,
                nodeId: neighbor.id,
                axis,
            })
        })
    }

    return null
}

function buildFallbackRoute(
    sourceAnchor: EdgeRoutePoint,
    sourceLead: EdgeRoutePoint,
    targetLead: EdgeRoutePoint,
    targetAnchor: EdgeRoutePoint,
): EdgeRoute {
    const midpointX = (sourceLead.x + targetLead.x) / 2
    const midpointY = (sourceLead.y + targetLead.y) / 2

    if (sourceLead.x === targetLead.x || sourceLead.y === targetLead.y) {
        return normalizeRoute([
            sourceAnchor,
            sourceLead,
            targetLead,
            targetAnchor,
        ])
    }

    if (Math.abs(sourceLead.x - targetLead.x) >= Math.abs(sourceLead.y - targetLead.y)) {
        return normalizeRoute([
            sourceAnchor,
            sourceLead,
            { x: midpointX, y: sourceLead.y },
            { x: midpointX, y: targetLead.y },
            targetLead,
            targetAnchor,
        ])
    }

    return normalizeRoute([
        sourceAnchor,
        sourceLead,
        { x: sourceLead.x, y: midpointY },
        { x: targetLead.x, y: midpointY },
        targetLead,
        targetAnchor,
    ])
}

// A small loop just outside the node: out of the source port, round to the target port.
function buildSelfLoopRoute(rect: NodeRect, sourcePort: RoutedPort, targetPort: RoutedPort): EdgeRoute {
    const sourceAnchor = getPortAnchorPoint(rect, sourcePort)
    const targetAnchor = getPortAnchorPoint(rect, targetPort)
    const sourceLead = getPortLeadPoint(sourceAnchor, sourcePort.side)
    const targetLead = getPortLeadPoint(targetAnchor, targetPort.side)
    const isHorizontal = (side: RouteSide) => side === 'left' || side === 'right'

    if (isHorizontal(sourcePort.side) === isHorizontal(targetPort.side) && sourcePort.side !== targetPort.side) {
        // Opposite sides: wrap over the top, or round the right.
        const corners = isHorizontal(sourcePort.side)
            ? [{ x: sourceLead.x, y: rect.y - PORT_LEAD_DISTANCE }, { x: targetLead.x, y: rect.y - PORT_LEAD_DISTANCE }]
            : [{ x: rect.x + rect.width + PORT_LEAD_DISTANCE, y: sourceLead.y }, { x: rect.x + rect.width + PORT_LEAD_DISTANCE, y: targetLead.y }]
        return normalizeRoute([sourceAnchor, sourceLead, ...corners, targetLead, targetAnchor])
    }

    // Same or adjacent sides: one corner joins the two leads (it is the target lead when the sides match).
    const corner = isHorizontal(sourcePort.side)
        ? { x: sourceLead.x, y: targetLead.y }
        : { x: targetLead.x, y: sourceLead.y }
    return normalizeRoute([sourceAnchor, sourceLead, corner, targetLead, targetAnchor])
}

function routeSingleEdge(
    edge: FixedNodeRouterEdge,
    nodeMap: Map<string, NodeRect>,
    expandedObstacles: ExpandedRect[],
    routedSegments: PreviousRouteSegment[],
): EdgeRoute {
    const sourceRect = nodeMap.get(edge.source)
    const targetRect = nodeMap.get(edge.target)
    if (!sourceRect || !targetRect) {
        return normalizeRoute(edge.previousRoute ?? [])
    }

    if (edge.source === edge.target) {
        return buildSelfLoopRoute(sourceRect, edge.sourcePort, edge.targetPort)
    }

    const sourceAnchor = getPortAnchorPoint(sourceRect, edge.sourcePort)
    const targetAnchor = getPortAnchorPoint(targetRect, edge.targetPort)

    const sourceLead = getPortEscapePoint(sourceAnchor, edge.sourcePort.side, expandedObstacles)
    const targetLead = getPortEscapePoint(targetAnchor, edge.targetPort.side, expandedObstacles)

    const xCoordinates = new Set<number>([
        sourceAnchor.x,
        sourceLead.x,
        targetLead.x,
        targetAnchor.x,
    ])
    const yCoordinates = new Set<number>([
        sourceAnchor.y,
        sourceLead.y,
        targetLead.y,
        targetAnchor.y,
    ])

    edge.previousRoute?.forEach((point) => {
        xCoordinates.add(point.x)
        yCoordinates.add(point.y)
    })
    expandedObstacles.forEach((obstacle) => {
        xCoordinates.add(obstacle.x)
        xCoordinates.add(obstacle.x + obstacle.width)
        yCoordinates.add(obstacle.y)
        yCoordinates.add(obstacle.y + obstacle.height)
    })

    const xs = [...xCoordinates].sort(compareNumber)
    const ys = [...yCoordinates].sort(compareNumber)
    const graphNodes = buildGraphNodes(xs, ys, expandedObstacles, [sourceLead, targetLead])
    const adjacency = buildAdjacency(graphNodes, xs, ys, expandedObstacles)
    const pathBetweenLeads = findShortestOrthogonalPath(
        graphNodes,
        adjacency,
        createPointId(sourceLead),
        createPointId(targetLead),
        edge.previousRoute,
        routedSegments,
    )

    if (!pathBetweenLeads) {
        return buildFallbackRoute(sourceAnchor, sourceLead, targetLead, targetAnchor)
    }

    return normalizeRoute([
        sourceAnchor,
        ...pathBetweenLeads,
        targetAnchor,
    ])
}

type RouteSegment = { edgeId: string; index: number; axis: Axis; min: number; max: number }

// Edges are routed one at a time, so several can run along the same corridor line and read as
// one. Spread each run of collinear, overlapping segments a few pixels apart, around the line,
// so every edge stays traceable. Only segments between two bends move; the spread stays well
// inside the obstacle padding, so no edge is pushed into a node.
export function separateOverlappingSegments(routes: Record<string, EdgeRoute>): Record<string, EdgeRoute> {
    const separated = Object.fromEntries(
        Object.entries(routes).map(([edgeId, route]) => [edgeId, route.map((point) => ({ ...point }))]),
    )
    const lines = new Map<string, RouteSegment[]>()
    Object.entries(separated).forEach(([edgeId, route]) => {
        for (let index = 1; index < route.length - 2; index += 1) {
            const from = route[index]
            const to = route[index + 1]
            const axis: Axis | null = from.y === to.y && from.x !== to.x ? 'horizontal' : from.x === to.x && from.y !== to.y ? 'vertical' : null
            if (!axis) {
                continue
            }
            const [min, max] = axis === 'horizontal' ? [Math.min(from.x, to.x), Math.max(from.x, to.x)] : [Math.min(from.y, to.y), Math.max(from.y, to.y)]
            const key = `${axis}:${axis === 'horizontal' ? from.y : from.x}`
            lines.set(key, [...(lines.get(key) ?? []), { edgeId, index, axis, min, max }])
        }
    })

    const spread = (run: RouteSegment[]) => {
        if (new Set(run.map((segment) => segment.edgeId)).size < 2) {
            return
        }
        const gap = Math.min(SEGMENT_SEPARATION, (2 * MAX_SEGMENT_SHIFT) / (run.length - 1))
        run.forEach((segment, position) => {
            const shift = (position - (run.length - 1) / 2) * gap
            const route = separated[segment.edgeId]
            const coordinate = segment.axis === 'horizontal' ? 'y' : 'x'
            route[segment.index][coordinate] += shift
            route[segment.index + 1][coordinate] += shift
        })
    }
    lines.forEach((segments) => {
        let run: RouteSegment[] = []
        let runEnd = Number.NEGATIVE_INFINITY
        ;[...segments]
            .sort((left, right) => left.min - right.min || left.edgeId.localeCompare(right.edgeId))
            .forEach((segment) => {
                if (segment.min >= runEnd - EPSILON) {
                    spread(run)
                    run = []
                }
                run.push(segment)
                runEnd = run.length === 1 ? segment.max : Math.max(runEnd, segment.max)
            })
        spread(run)
    })
    return separated
}

export function routeFixedNodeGraph(request: FixedNodeRouterRequest): FixedNodeRouterResult {
    const nodeMap = new Map(
        request.nodes.map((node) => [node.id, node.rect]),
    )
    const expandedObstacles = request.nodes.map((node) => expandRect(node.rect, OBSTACLE_PADDING))
    // Shortest edges first, so the long ones, such as routes to a shared exit, bend around them.
    const span = (edge: FixedNodeRouterEdge) => {
        const source = nodeMap.get(edge.source)
        const target = nodeMap.get(edge.target)
        return source && target
            ? Math.abs(source.x + source.width / 2 - target.x - target.width / 2) + Math.abs(source.y + source.height / 2 - target.y - target.height / 2)
            : 0
    }
    const routedSegments: PreviousRouteSegment[] = []
    const routesById = new Map<string, EdgeRoute>()
    ;[...request.edges]
        .sort((left, right) => span(left) - span(right))
        .forEach((edge) => {
            const route = routeSingleEdge(edge, nodeMap, expandedObstacles, routedSegments)
            routesById.set(edge.id, route)
            routedSegments.push(...buildPreviousRouteSegments(route))
        })
    const routes = Object.fromEntries(request.edges.map((edge) => [edge.id, routesById.get(edge.id) ?? []]))
    return { routes: separateOverlappingSegments(routes) }
}
