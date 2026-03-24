import type { HTMLAttributes, ReactNode } from "react"

import { cn } from "@/lib/utils"

interface EmptyStateProps extends HTMLAttributes<HTMLDivElement> {
  title?: string | null
  description: ReactNode
}

function EmptyState({ title, description, className, ...props }: EmptyStateProps) {
  return (
    <div
      {...props}
      data-slot="empty-state"
      className={cn("rounded-md border border-dashed border-border px-3 py-4 text-sm text-muted-foreground", className)}
    >
      {title ? <p className="mb-1 font-medium text-foreground">{title}</p> : null}
      <div>{description}</div>
    </div>
  )
}

export { EmptyState }
