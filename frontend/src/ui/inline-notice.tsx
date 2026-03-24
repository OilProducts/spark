import type { HTMLAttributes, ReactNode } from "react"

import { cn } from "@/lib/utils"

const TONE_CLASS_NAME: Record<"neutral" | "warning" | "error" | "success", string> = {
  neutral: "border-border/70 bg-muted/20 text-muted-foreground",
  warning: "border-amber-500/40 bg-amber-500/10 text-amber-800",
  error: "border-destructive/40 bg-destructive/10 text-destructive",
  success: "border-emerald-500/40 bg-emerald-500/10 text-emerald-800",
}

interface InlineNoticeProps extends HTMLAttributes<HTMLDivElement> {
  tone?: "neutral" | "warning" | "error" | "success"
  children: ReactNode
}

function InlineNotice({ tone = "neutral", className, children, ...props }: InlineNoticeProps) {
  return (
    <div
      {...props}
      data-slot="inline-notice"
      className={cn("rounded-md border px-3 py-2 text-sm", TONE_CLASS_NAME[tone], className)}
    >
      {children}
    </div>
  )
}

export { InlineNotice }
