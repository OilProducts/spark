export const normalizeProjectPath = (path: string): string => {
  const trimmed = path.trim()
  if (!trimmed) return ""
  const slashNormalized = trimmed.replace(/\\/g, "/").replace(/\/{2,}/g, "/")
  const windowsPrefixMatch = slashNormalized.match(/^[A-Za-z]:\//)
  const prefix = slashNormalized.startsWith("/") ? "/" : windowsPrefixMatch ? windowsPrefixMatch[0] : ""
  const rawBody = prefix ? slashNormalized.slice(prefix.length) : slashNormalized
  const parts = rawBody.split("/").filter((part) => part.length > 0)
  const segments: string[] = []
  for (const part of parts) {
    if (part === ".") {
      continue
    }
    if (part === "..") {
      if (segments.length > 0) {
        segments.pop()
      }
      continue
    }
    segments.push(part)
  }
  const normalizedBody = segments.join("/")
  if (!normalizedBody) {
    if (prefix === "/") {
      return "/"
    }
    return prefix || normalizedBody
  }
  return `${prefix}${normalizedBody}`
}

export const isAbsoluteProjectPath = (path: string): boolean => path.startsWith("/") || /^[A-Za-z]:\//.test(path)

export const formatProjectPathLabel = (projectPath: string | null): string => {
  if (!projectPath) {
    return "No active project"
  }

  const normalizedPath = normalizeProjectPath(projectPath)
  const segments = normalizedPath.split("/").filter(Boolean)
  return segments[segments.length - 1] || normalizedPath
}
