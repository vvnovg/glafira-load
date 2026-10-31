/** Абсолютный путь клона: `rel` относительно папки проекта или как есть, если абсолютный (POSIX или Windows). */
export function joinProjectPath(project: string, rel: string): string {
  if (/^(?:[a-zA-Z]:[\\/]|\\\\|\/)/.test(rel)) return rel;
  const sep = project.includes("\\") && !project.includes("/") ? "\\" : "/";
  return `${project.replace(/[\\/]+$/, "")}${sep}${rel.replace(/^[\\/]+/, "")}`;
}

/** Имя папки из пути. */
export function baseName(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
}

/** Путь для манифеста: внутри папки проекта — относительный с `/`, снаружи — абсолютный как есть. */
export function projectRelative(project: string, abs: string): string {
  const root = project.replace(/[\\/]+$/, "");
  if (abs.length <= root.length + 1 || !abs.startsWith(root) || !/[\\/]/.test(abs[root.length])) return abs;
  return abs.slice(root.length + 1).split(/[\\/]+/).filter(Boolean).join("/");
}
