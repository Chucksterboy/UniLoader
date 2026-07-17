import path from "node:path";

export function normalizeArchivePath(input: string): string {
  return input.replace(/\\/g, "/").replace(/^\/+/, "");
}

export function getCommonTopFolder(paths: string[]): string | null {
  const filePaths = paths
    .map(normalizeArchivePath)
    .filter((entryPath) => entryPath && !entryPath.endsWith("/"));

  if (filePaths.length === 0) {
    return null;
  }

  const firstParts = filePaths[0].split("/");
  if (firstParts.length < 2) {
    return null;
  }

  const candidate = firstParts[0];
  const everyPathSharesTopFolder = filePaths.every((entryPath) =>
    entryPath.startsWith(`${candidate}/`)
  );

  return everyPathSharesTopFolder ? candidate : null;
}

export function toLogicalPath(entryPath: string, commonTopFolder: string | null): string {
  const normalized = normalizeArchivePath(entryPath);
  if (!commonTopFolder) {
    return normalized;
  }

  const prefix = `${commonTopFolder}/`;
  return normalized.startsWith(prefix) ? normalized.slice(prefix.length) : normalized;
}

export function hasExtension(entryPath: string, extensions: string[]): boolean {
  const lowerPath = entryPath.toLowerCase();
  return extensions.some((extension) => lowerPath.endsWith(extension.toLowerCase()));
}

export function includesPathSegment(entryPath: string, segment: string): boolean {
  return normalizeArchivePath(entryPath)
    .toLowerCase()
    .split("/")
    .includes(segment.toLowerCase());
}

export function safeJoin(root: string, relativePath: string): string {
  const normalizedRelativePath = normalizeArchivePath(relativePath);
  if (
    normalizedRelativePath.includes("../") ||
    normalizedRelativePath === ".." ||
    path.isAbsolute(normalizedRelativePath)
  ) {
    throw new Error(`Unsafe archive path: ${relativePath}`);
  }

  const targetPath = path.resolve(root, normalizedRelativePath);
  const resolvedRoot = path.resolve(root);

  if (targetPath !== resolvedRoot && !targetPath.startsWith(`${resolvedRoot}${path.sep}`)) {
    throw new Error(`Install target escaped root: ${relativePath}`);
  }

  return targetPath;
}

export function basenameWithoutArchiveExtension(archivePath: string): string {
  return path.basename(archivePath).replace(/\.(zip|7z|rar)$/i, "");
}
