export const normalizeProjectPath = (path: string): string => {
  const trimmed = path.trim()
  if (!trimmed) return ""
  const forwardSlashed = trimmed.replace(/\\/g, "/")
  // A UNC path (\\server\share) keeps `//server/share` as its root, so `..` cannot climb above the share.
  const unc = forwardSlashed.match(/^\/\/([^/]+)\/*([^/]*)(.*)$/)
  const uncRoot = unc ? `//${unc[1]}${unc[2] ? `/${unc[2]}` : ""}` : null
  const slashNormalized = forwardSlashed.replace(/\/{2,}/g, "/")
  const windowsPrefixMatch = slashNormalized.match(/^[A-Za-z]:\//)
  const prefix = slashNormalized.startsWith("/") ? "/" : windowsPrefixMatch ? windowsPrefixMatch[0] : ""
  const rawBody = unc ? unc[3] : prefix ? slashNormalized.slice(prefix.length) : slashNormalized
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
  if (uncRoot) {
    return normalizedBody ? `${uncRoot}/${normalizedBody}` : uncRoot
  }
  if (!normalizedBody) {
    if (prefix === "/") {
      return "/"
    }
    return prefix || normalizedBody
  }
  return `${prefix}${normalizedBody}`
}

// POSIX (/x), drive (C:/x or C:\x) or UNC (//server/share or \\server\share).
export const isAbsoluteProjectPath = (path: string): boolean => /^(?:\/|\\\\|[A-Za-z]:[/\\])/.test(path)

export const formatProjectPathLabel = (projectPath: string | null): string => {
  if (!projectPath) {
    return "No active project"
  }

  const normalizedPath = normalizeProjectPath(projectPath)
  const segments = normalizedPath.split("/").filter(Boolean)
  return segments[segments.length - 1] || normalizedPath
}
