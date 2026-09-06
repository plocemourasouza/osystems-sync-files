/**
 * FileTypeIcon — extension-based glyph for `JobRow`'s "Arquivo & Origem"
 * cell (PRD.md RF-062; DESIGN.md §7 icon list: `description`/`FileText`
 * family). Maps the file's last extension to a `lucide-react` icon,
 * falling back to a generic `File` for anything unrecognized.
 */
import { Archive, Database, File, FileCode2, FileText, Film, Table2, type LucideIcon } from "lucide-react";
import type { JSX } from "react";

const EXTENSION_ICON: Record<string, LucideIcon> = {
  sql: Database,
  dump: Database,
  db: Database,
  zip: Archive,
  tar: Archive,
  gz: Archive,
  "7z": Archive,
  mp4: Film,
  mov: Film,
  mkv: Film,
  yml: FileCode2,
  yaml: FileCode2,
  json: FileCode2,
  toml: FileCode2,
  parquet: Table2,
  csv: Table2,
  log: FileText,
  txt: FileText,
};

function extensionOf(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  return dotIndex === -1 ? "" : name.slice(dotIndex + 1).toLowerCase();
}

export type FileTypeIconProps = {
  name: string;
  size?: number;
  className?: string;
};

export function FileTypeIcon({ name, size = 16, className }: FileTypeIconProps): JSX.Element {
  const Icon = EXTENSION_ICON[extensionOf(name)] ?? File;
  return <Icon aria-hidden="true" size={size} className={className} />;
}
