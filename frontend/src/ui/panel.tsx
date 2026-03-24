import * as React from "react"

import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/ui/card"
import { cn } from "@/lib/utils"

function Panel({
  className,
  children,
  ...props
}: React.ComponentProps<typeof Card>) {
  return (
    <Card className={cn("gap-4 py-4 shadow-sm", className)} {...props}>
      {children}
    </Card>
  )
}

function PanelHeader({
  className,
  ...props
}: React.ComponentProps<typeof CardHeader>) {
  return <CardHeader className={cn("gap-1 px-4", className)} {...props} />
}

function PanelTitle({
  className,
  ...props
}: React.ComponentProps<typeof CardTitle>) {
  return <CardTitle className={cn("text-sm", className)} {...props} />
}

function PanelDescription({
  className,
  ...props
}: React.ComponentProps<typeof CardDescription>) {
  return <CardDescription className={cn("text-xs", className)} {...props} />
}

function PanelContent({
  className,
  ...props
}: React.ComponentProps<typeof CardContent>) {
  return <CardContent className={cn("px-4", className)} {...props} />
}

export { Panel, PanelContent, PanelDescription, PanelHeader, PanelTitle }
