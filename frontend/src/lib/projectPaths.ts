export const normalizeProjectPath = (path: string): string => {
  const trimmed = path.trim()
  if (!trimmed) return ""
  const forwardSlashed = trimmed.replace(/\\/g, "/")
  // A UNC path (\\server\share) keeps `//server/share` as its root, so `..` cannot climb above the share.
  // ponytail: requests send this key, and the backend's normalize_path keeps a UNC prefix exactly as spelled
  // (crates/spark-common/src/paths.rs), so a project it registered as \\server\share is another identity.
  // The owner deferred this to a backend fix that writes the UNC prefix in one spelling, not a frontend rewrite of every request.
  // No `.` here: it skips line separators (U+2028/9), which are valid in folder names.
  const unc = forwardSlashed.match(/^\/\/([^/]+)\/*([^/]*)/)
  const uncRoot = unc ? `//${unc[1]}${unc[2] ? `/${unc[2]}` : ""}` : null
  const slashNormalized = forwardSlashed.replace(/\/{2,}/g, "/")
  const windowsPrefixMatch = slashNormalized.match(/^[A-Za-z]:\//)
  const prefix = slashNormalized.startsWith("/") ? "/" : windowsPrefixMatch ? windowsPrefixMatch[0] : ""
  const rawBody = unc ? forwardSlashed.slice(unc[0].length) : prefix ? slashNormalized.slice(prefix.length) : slashNormalized
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
