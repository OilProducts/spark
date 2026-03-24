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

export type ElkEdgeSectionLike = {
    id?: string
    startPoint: EdgeRoutePoint
    endPoint: EdgeRoutePoint
    bendPoints?: EdgeRoutePoint[]
    incomingSections?: string[]
    outgoingSections?: string[]
}

type RouteSide = 'top' | 'right' | 'bottom' | 'left'

const SAME_ROW_THRESHOLD = 72
const LOOPBACK_CLEARANCE = 40
const EDGE_CORNER_RADIUS = 12

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

function normalizeRoute(route: EdgeRoute): EdgeRoute {
    const deduped = route.filter(isFinitePoint).reduce<EdgeRoute>((points, point) => {
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

function orderElkSections(sections: readonly ElkEdgeSectionLike[]): ElkEdgeSectionLike[] {
    if (sections.length <= 1) {
        return [...sections]
    }

    const sectionById = new Map(
        sections
            .filter((section): section is ElkEdgeSectionLike & { id: string } => typeof section.id === 'string')
            .map((section) => [section.id, section]),
    )
    const startSection = sections.find((section) => !section.incomingSections?.length) ?? sections[0]
    const ordered: ElkEdgeSectionLike[] = []
    const visited = new Set<string>()
    let current: ElkEdgeSectionLike | undefined = startSection

    while (current) {
        if (current.id) {
            if (visited.has(current.id)) {
                break
            }
            visited.add(current.id)
        }
        ordered.push(current)
        const nextSectionId: string | undefined = current.outgoingSections?.find((sectionId) => sectionById.has(sectionId))
        current = nextSectionId ? sectionById.get(nextSectionId) : undefined
    }

    sections.forEach((section) => {
        if (!section.id || !visited.has(section.id)) {
            ordered.push(section)
        }
    })

    return ordered
}

export function flattenElkSectionToRoute(
    sections?: readonly ElkEdgeSectionLike[] | null,
): EdgeRoute | null {
    if (!sections?.length) {
        return null
    }

    const orderedSections = orderElkSections(sections)
    const route = orderedSections.flatMap((section) => [
        section.startPoint,
        ...(section.bendPoints ?? []),
        section.endPoint,
    ])
    const normalizedRoute = normalizeRoute(route)
    return normalizedRoute.length >= 2 ? normalizedRoute : null
}

function getRectCenter(rect: NodeRect): EdgeRoutePoint {
    return {
        x: rect.x + rect.width / 2,
        y: rect.y + rect.height / 2,
    }
}

function getAnchorPoint(rect: NodeRect, side: RouteSide): EdgeRoutePoint {
    if (side === 'top') {
        return { x: rect.x + rect.width / 2, y: rect.y }
    }
    if (side === 'right') {
        return { x: rect.x + rect.width, y: rect.y + rect.height / 2 }
    }
    if (side === 'bottom') {
        return { x: rect.x + rect.width / 2, y: rect.y + rect.height }
    }
    return { x: rect.x, y: rect.y + rect.height / 2 }
}

function buildMidpointRoute(start: EdgeRoutePoint, end: EdgeRoutePoint, verticalFirst: boolean): EdgeRoute {
    if (verticalFirst) {
        const midpointY = (start.y + end.y) / 2
        return normalizeRoute([
            start,
            { x: start.x, y: midpointY },
            { x: end.x, y: midpointY },
            end,
        ])
    }

    const midpointX = (start.x + end.x) / 2
    return normalizeRoute([
        start,
        { x: midpointX, y: start.y },
        { x: midpointX, y: end.y },
        end,
    ])
}

function buildLoopbackRoute(start: EdgeRoutePoint, end: EdgeRoutePoint, side: Extract<RouteSide, 'left' | 'right'>): EdgeRoute {
    const routeX = side === 'left'
        ? Math.min(start.x, end.x) - LOOPBACK_CLEARANCE
        : Math.max(start.x, end.x) + LOOPBACK_CLEARANCE

    return normalizeRoute([
        start,
        { x: routeX, y: start.y },
        { x: routeX, y: end.y },
        end,
    ])
}

export function buildFallbackOrthogonalRoute(sourceRect: NodeRect, targetRect: NodeRect): EdgeRoute {
    const sourceCenter = getRectCenter(sourceRect)
    const targetCenter = getRectCenter(targetRect)
    const dx = targetCenter.x - sourceCenter.x
    const dy = targetCenter.y - sourceCenter.y

    if (Math.abs(dy) <= SAME_ROW_THRESHOLD) {
        const sourceSide: RouteSide = dx >= 0 ? 'right' : 'left'
        const targetSide: RouteSide = dx >= 0 ? 'left' : 'right'
        return buildMidpointRoute(
            getAnchorPoint(sourceRect, sourceSide),
            getAnchorPoint(targetRect, targetSide),
            false,
        )
    }

    if (dy > 0) {
        return buildMidpointRoute(
            getAnchorPoint(sourceRect, 'bottom'),
            getAnchorPoint(targetRect, 'top'),
            true,
        )
    }

    if (Math.abs(dx) < Math.max(sourceRect.width, targetRect.width)) {
        const side: Extract<RouteSide, 'left' | 'right'> = dx <= 0 ? 'left' : 'right'
        return buildLoopbackRoute(
            getAnchorPoint(sourceRect, side),
            getAnchorPoint(targetRect, side),
            side,
        )
    }

    const sourceSide: RouteSide = dx >= 0 ? 'right' : 'left'
    const targetSide: RouteSide = dx >= 0 ? 'left' : 'right'
    return buildMidpointRoute(
        getAnchorPoint(sourceRect, sourceSide),
        getAnchorPoint(targetRect, targetSide),
        false,
    )
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

export function buildPolylinePath(route: EdgeRoute | null | undefined): string {
    const normalizedRoute = normalizeRoute(route ?? [])
    if (!normalizedRoute.length) {
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
    const normalizedRoute = normalizeRoute(route ?? [])
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

export function stripEdgeLayoutRoutes<EdgeType extends { data?: Record<string, unknown> | undefined }>(
    edges: EdgeType[],
): EdgeType[] {
    let mutated = false

    const nextEdges = edges.map((edge) => {
        const edgeData = edge.data
        if (!edgeData || !('layoutRoute' in edgeData)) {
            return edge
        }

        mutated = true
        const { layoutRoute: _layoutRoute, ...rest } = edgeData
        if (Object.keys(rest).length === 0) {
            return { ...edge, data: undefined }
        }
        return { ...edge, data: rest }
    })

    return mutated ? nextEdges : edges
}
