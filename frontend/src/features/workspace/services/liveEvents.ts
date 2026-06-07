import { workspaceLiveEventsUrl } from '@/lib/api/apiClient'

export function buildWorkspaceLiveEventsUrl(params: URLSearchParams): string {
    return workspaceLiveEventsUrl(params)
}
