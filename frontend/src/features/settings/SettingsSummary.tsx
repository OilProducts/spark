import type { ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

// A <details> summary with the app's turning chevron in place of the browser triangle.
// ponytail: native <details>, not Collapsible: settings tests and smoke specs select details/summary.
export function SettingsSummary({ children }: { children: ReactNode }) {
    return <summary className="flex items-center gap-1.5 text-sm font-medium text-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight aria-hidden="true" className="size-4 shrink-0 text-muted-foreground transition-transform [details[open]>summary>&]:rotate-90" />
        {children}
    </summary>
}
