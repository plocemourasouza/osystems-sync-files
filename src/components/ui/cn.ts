/**
 * cn — minimal className joiner (no `clsx` dependency, CLAUDE.md "no new deps").
 *
 * Accepts strings and falsy values (from conditional expressions like
 * `condition && "class"`), filters out the falsy ones, and joins what's left
 * with a single space. Intentionally does not dedupe or merge conflicting
 * Tailwind utilities (that's `tailwind-merge`'s job, not needed here — every
 * primitive in this folder composes a small, non-conflicting class list).
 */
export type ClassValue = string | false | null | undefined;

export function cn(...classes: ClassValue[]): string {
  return classes.filter((value): value is string => Boolean(value)).join(" ");
}
