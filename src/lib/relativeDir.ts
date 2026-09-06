/**
 * relativeDir — a job's file directory, shown relative to the watched root
 * (PRD.md RF-070 folder picker; DESIGN.md §8 JobRow "Origem" sub-line).
 *
 * A real 1366×768 Windows run showed the raw NT "verbatim" path form
 * (`\\?\C:\monitoramento`, which crosses the Tauri IPC boundary as
 * `//?/C:/monitoramento`) in the table. This strips that prefix defensively
 * — even though the backend is expected to normalize it separately — and
 * always renders the folder relative to `watch.path` instead of the
 * absolute path, since the operator already knows their own watched folder.
 */

function stripVerbatimPrefix(path: string): string {
  return path.replace(/^\\\\\?\\/, "").replace(/^\/\/\?\//, "");
}

function normalizeSlashes(path: string): string {
  return path.replace(/\\/g, "/");
}

function dirnameOf(path: string): string {
  const lastSlash = path.lastIndexOf("/");
  return lastSlash === -1 ? "" : path.slice(0, lastSlash);
}

function stripTrailingSlash(path: string): string {
  return path.length > 1 && path.endsWith("/") ? path.slice(0, -1) : path;
}

/**
 * `path`'s directory, relative to `root` (`configStore`'s `watch.path`).
 *
 * - `root === null` (no folder configured yet): falls back to `path`'s raw,
 *   verbatim-prefix-stripped, forward-slash-normalized directory.
 * - the file sits directly in `root`: `"./"`.
 * - the file sits in a subfolder of `root`: `"./<sub>/<dirs>"`.
 * - the file is outside `root` (shouldn't normally happen): the raw
 *   normalized directory, same as the `root === null` case.
 *
 * Comparison against `root` is case-insensitive (Windows paths), but the
 * returned sub-path preserves the original casing.
 */
export function relativeDir(path: string, root: string | null): string {
  const fileDir = dirnameOf(normalizeSlashes(stripVerbatimPrefix(path)));

  if (root === null || root === "") return fileDir;

  const normalizedRoot = stripTrailingSlash(normalizeSlashes(stripVerbatimPrefix(root)));

  if (fileDir.toLowerCase() === normalizedRoot.toLowerCase()) return "./";

  const prefix = `${normalizedRoot.toLowerCase()}/`;
  if (fileDir.toLowerCase().startsWith(prefix)) {
    return `./${fileDir.slice(prefix.length)}`;
  }

  return fileDir;
}
