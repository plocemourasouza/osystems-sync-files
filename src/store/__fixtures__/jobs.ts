/**
 * Test fixtures for `JobView`/`JobSide` (SPEC.md §7). Shared by `jobsStore` and
 * component tests that need a valid row without hand-writing every field.
 */
import type { JobSide, JobView } from "@/types/generated";

let sequence = 0;

function nextId(prefix: string): string {
  sequence += 1;
  return `${prefix}-${sequence}`;
}

export function makeJobSide(overrides: Partial<JobSide> = {}): JobSide {
  return {
    job_id: nextId("job"),
    status: "pending",
    attempts: 0,
    next_attempt_at: null,
    remote_id: null,
    last_error: null,
    updated_at: "2026-01-01T00:00:00.000Z",
    ...overrides,
  };
}

interface MakeJobOverrides extends Partial<Omit<JobView, "gdrive" | "s3">> {
  gdrive?: Partial<JobSide>;
  s3?: Partial<JobSide>;
}

/** Builds a valid `JobView`, letting callers override any top-level field or either side. */
export function makeJob(overrides: MakeJobOverrides = {}): JobView {
  const { gdrive, s3, ...rest } = overrides;
  const id = nextId("file");

  return {
    file_id: id,
    path: `C:\\Watch\\${id}.pdf`,
    name: `${id}.pdf`,
    size: 1024,
    sha256: "0".repeat(64),
    detected_at: "2026-01-01T00:00:00.000Z",
    ...rest,
    gdrive: makeJobSide(gdrive),
    s3: makeJobSide(s3),
  };
}
