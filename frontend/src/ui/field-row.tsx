import type { ReactNode } from "react"

import { cn } from "@/lib/utils"
import { Label } from "@/ui/label"

interface FieldRowProps {
  label: string
  htmlFor?: string
  helper?: string | null
  error?: string | null
  className?: string
  children: ReactNode
}

function FieldRow({ label, htmlFor, helper, error, className, children }: FieldRowProps) {
  return (
    <div className={cn("space-y-1", className)}>
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
      {helper ? <p className="text-[11px] text-muted-foreground">{helper}</p> : null}
      {error ? <p className="text-[11px] text-destructive">{error}</p> : null}
    </div>
  )
}

export { FieldRow }
