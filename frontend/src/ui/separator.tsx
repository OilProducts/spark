import * as React from "react"

import { cn } from "@/lib/utils"

function Separator({
  className,
  orientation = "horizontal",
  ...props
}: React.ComponentProps<"div"> & {
  orientation?: "horizontal" | "vertical"
}) {
  return (
    <div
      data-slot="separator"
      aria-hidden="true"
      className={cn(
        orientation === "horizontal" ? "h-px w-full bg-border" : "h-full w-px bg-border",
        className
      )}
      {...props}
    />
  )
}

export { Separator }
