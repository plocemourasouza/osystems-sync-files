#!/usr/bin/env node
// tests/e2e/longevity.mjs
//
// 72-hour longevity test harness for osystems-sync (T-6.1 / T-6.6).
//
// Two modes:
//   1. generator (default): drops one file/minute into a watched folder,
//      alternating sizes, and samples the app's RSS + SQLite queue state
//      every few minutes into a CSV. Writes a JSON manifest of every file
//      it created (path, size, sha256) so a later report can prove nothing
//      was lost or duplicated.
//   2. `--mode report`: reads a manifest + CSV (and optionally newline
//      "name sha256" listings scraped from S3 / Google Drive) and renders
//      tests/e2e/longevity-report.md with PASS/FAIL against RNF-001
//      (RSS drift <= 10%) and RF-039 (never the same file twice at the
//      same destination -> zero duplicates).
//
// No dependencies. Node built-ins only (node:fs, node:child_process,
// node:crypto, node:os, node:path). SQLite access shells out to the
// `sqlite3` CLI when present; otherwise DB-derived columns are skipped
// with a warning (never silently faked).
//
// See tests/e2e/README.md for usage.

import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import crypto from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

/**
 * Minimal CLI parser: `--flag value` pairs and bare boolean `--flag`s.
 * Returns { mode, ...options } with defaults applied.
 */
export function parseArgs(argv) {
  const opts = {
    mode: "generate",
    dir: null,
    hours: 72,
    minutes: null, // dry-run override for `hours`
    intervalMin: 1,
    intervalSec: null, // dry-run override for `intervalMin`
    sampleIntervalSec: 300, // 5 min
    processName: "osystems-sync",
    out: null,
    manifest: null,
    sizes: "16KB,512KB,4MB,64MB,256MB",
    seed: 42,
    dryRun: false,
    db: null,
    csv: null,
    s3List: null,
    driveList: null,
    reportOut: null,
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (!arg.startsWith("--")) continue;
    const key = arg.slice(2);
    const next = argv[i + 1];
    const isBoolean = key === "dry-run";
    switch (key) {
      case "mode":
        opts.mode = next;
        i++;
        break;
      case "dir":
        opts.dir = next;
        i++;
        break;
      case "hours":
        opts.hours = Number(next);
        i++;
        break;
      case "minutes":
        opts.minutes = Number(next);
        i++;
        break;
      case "interval-min":
        opts.intervalMin = Number(next);
        i++;
        break;
      case "interval-sec":
        opts.intervalSec = Number(next);
        i++;
        break;
      case "sample-interval-sec":
        opts.sampleIntervalSec = Number(next);
        i++;
        break;
      case "process-name":
        opts.processName = next;
        i++;
        break;
      case "out":
        opts.out = next;
        i++;
        break;
      case "manifest":
        opts.manifest = next;
        i++;
        break;
      case "sizes":
        opts.sizes = next;
        i++;
        break;
      case "seed":
        opts.seed = Number(next);
        i++;
        break;
      case "dry-run":
        opts.dryRun = true;
        break;
      case "db":
        opts.db = next;
        i++;
        break;
      case "csv":
        opts.csv = next;
        i++;
        break;
      case "s3-list":
        opts.s3List = next;
        i++;
        break;
      case "drive-list":
        opts.driveList = next;
        i++;
        break;
      case "report-out":
        opts.reportOut = next;
        i++;
        break;
      default:
        if (!isBoolean) i++; // skip unknown option's value defensively
        break;
    }
  }
  return opts;
}

// ---------------------------------------------------------------------------
// Deterministic content generation (seeded xorshift32)
// ---------------------------------------------------------------------------

/** xorshift32 PRNG. Returns a function yielding the next uint32. */
export function xorshift32(seed) {
  let state = seed >>> 0 || 0x9e3779b9;
  return function next() {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state;
  };
}

/**
 * Per-file seed: mixes the run seed with the file index via a Weyl
 * increment so consecutive files never share a PRNG stream (and thus
 * never share content/sha256), while remaining fully deterministic.
 */
export function fileSeed(seed, n) {
  return (((seed >>> 0) ^ Math.imul(n + 1, 0x9e3779b1)) >>> 0) || 1;
}

/** Parses a human size string like "512KB", "4MB", "16KB" into bytes (binary units). */
export function parseSize(str) {
  const m = /^(\d+(?:\.\d+)?)\s*(B|KB|MB|GB)$/i.exec(str.trim());
  if (!m) throw new Error(`Invalid size: "${str}" (expected e.g. 16KB, 4MB)`);
  const n = parseFloat(m[1]);
  const unit = m[2].toUpperCase();
  const mult = { B: 1, KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3 }[unit];
  return Math.round(n * mult);
}

/** Parses "16KB,512KB,4MB" into [{ label, bytes }]. */
export function parseSizesList(str) {
  return str
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
    .map((label) => ({ label, bytes: parseSize(label) }));
}

const CHUNK_SIZE = 1024 * 1024; // 1MB, simulates a real copy in chunks

/**
 * Writes `totalBytes` of deterministic pseudo-random content to `filePath`
 * in CHUNK_SIZE pieces (simulating an incremental copy), hashing as it goes.
 * Returns the sha256 hex digest. Content is a pure function of (seed, n),
 * so re-running with the same seed reproduces byte-identical files, but
 * every file index produces a distinct stream (no accidental sha256 dedup).
 */
export function writeDeterministicFile(filePath, totalBytes, seed, n) {
  const rng = xorshift32(fileSeed(seed, n));
  const hash = crypto.createHash("sha256");
  const fd = fs.openSync(filePath, "w");
  try {
    let written = 0;
    while (written < totalBytes) {
      const size = Math.min(CHUNK_SIZE, totalBytes - written);
      const buf = Buffer.allocUnsafe(size);
      for (let i = 0; i < size; i += 4) {
        const v = rng();
        buf[i] = v & 0xff;
        if (i + 1 < size) buf[i + 1] = (v >>> 8) & 0xff;
        if (i + 2 < size) buf[i + 2] = (v >>> 16) & 0xff;
        if (i + 3 < size) buf[i + 3] = (v >>> 24) & 0xff;
      }
      fs.writeSync(fd, buf);
      hash.update(buf);
      written += size;
    }
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest("hex");
}

// ---------------------------------------------------------------------------
// RSS sampling (cross-platform)
// ---------------------------------------------------------------------------

let warnedNoProcess = false;
let warnedNoSqlite = false;

/** Returns RSS in KB for the first process matching `processName`, or null. */
export function sampleRss(processName, platform = process.platform) {
  try {
    if (platform === "win32") {
      const cmd = `(Get-Process -Name '${processName}' -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty WorkingSet64)`;
      const res = spawnSync("powershell", ["-NoProfile", "-Command", cmd], {
        encoding: "utf8",
      });
      const out = (res.stdout || "").trim();
      if (!out) return null;
      const bytes = Number(out);
      return Number.isFinite(bytes) ? Math.round(bytes / 1024) : null;
    }
    // macOS / Linux
    let pid = null;
    const pgrep = spawnSync("pgrep", ["-x", processName], { encoding: "utf8" });
    if (pgrep.status === 0 && pgrep.stdout.trim()) {
      pid = pgrep.stdout.trim().split("\n")[0];
    } else {
      const pgrepF = spawnSync("pgrep", ["-f", processName], { encoding: "utf8" });
      if (pgrepF.status === 0 && pgrepF.stdout.trim()) {
        pid = pgrepF.stdout.trim().split("\n")[0];
      }
    }
    if (!pid) {
      if (!warnedNoProcess) {
        console.warn(
          `[longevity] warning: process "${processName}" not found; RSS samples will be blank`,
        );
        warnedNoProcess = true;
      }
      return null;
    }
    const ps = spawnSync("ps", ["-o", "rss=", "-p", pid], { encoding: "utf8" });
    const kb = Number((ps.stdout || "").trim());
    return Number.isFinite(kb) ? kb : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// SQLite state (via the `sqlite3` CLI — no native driver dependency)
// ---------------------------------------------------------------------------

/** Default per-platform state.db path (see SPEC.md). Overridable via --db. */
export function defaultDbPath(platform = process.platform, env = process.env, home = os.homedir()) {
  if (platform === "win32") {
    const appData = env.APPDATA || path.join(home, "AppData", "Roaming");
    return path.join(appData, "osystems-sync", "state.db");
  }
  return path.join(home, "Library", "Application Support", "osystems-sync", "state.db");
}

let sqliteAvailable = null;
function hasSqlite3() {
  if (sqliteAvailable !== null) return sqliteAvailable;
  const res = spawnSync("sqlite3", ["-version"], { encoding: "utf8" });
  sqliteAvailable = res.status === 0;
  return sqliteAvailable;
}

/**
 * Returns { files_local, jobs_pending, jobs_uploading, jobs_done, jobs_failed }
 * or null if the sqlite3 CLI or the DB file is unavailable (warns once).
 */
export function getDbCounts(dbPath) {
  if (!hasSqlite3()) {
    if (!warnedNoSqlite) {
      console.warn(`[longevity] warning: "sqlite3" CLI not found; DB columns will be blank`);
      warnedNoSqlite = true;
    }
    return null;
  }
  if (!dbPath || !fs.existsSync(dbPath)) {
    if (!warnedNoSqlite) {
      console.warn(`[longevity] warning: state.db not found at "${dbPath}"; DB columns will be blank`);
      warnedNoSqlite = true;
    }
    return null;
  }
  try {
    const filesRes = spawnSync("sqlite3", ["-readonly", "-noheader", dbPath, "select count(*) from files;"], {
      encoding: "utf8",
    });
    const filesLocal = Number((filesRes.stdout || "0").trim()) || 0;

    const jobsRes = spawnSync(
      "sqlite3",
      ["-readonly", "-noheader", "-csv", dbPath, "select status, count(*) from jobs group by status;"],
      { encoding: "utf8" },
    );
    const counts = { jobs_pending: 0, jobs_uploading: 0, jobs_done: 0, jobs_failed: 0 };
    for (const line of (jobsRes.stdout || "").split("\n")) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      const [status, countStr] = trimmed.split(",");
      const key = `jobs_${status.replace(/"/g, "")}`;
      if (key in counts) counts[key] = Number(countStr) || 0;
    }
    return { files_local: filesLocal, ...counts };
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Generator mode
// ---------------------------------------------------------------------------

const CSV_HEADER = "ts,rss_kb,files_local,jobs_pending,jobs_uploading,jobs_done,jobs_failed";

function ensureCsvHeader(csvPath) {
  fs.mkdirSync(path.dirname(csvPath), { recursive: true });
  if (!fs.existsSync(csvPath)) {
    fs.writeFileSync(csvPath, CSV_HEADER + "\n");
  }
}

function appendCsvRow(csvPath, row) {
  const fields = [
    row.ts,
    row.rss_kb ?? "",
    row.files_local ?? "",
    row.jobs_pending ?? "",
    row.jobs_uploading ?? "",
    row.jobs_done ?? "",
    row.jobs_failed ?? "",
  ];
  fs.appendFileSync(csvPath, fields.join(",") + "\n");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function defaultOutPath() {
  const ts = new Date().toISOString().replace(/[:.]/g, "-");
  return path.join(__dirname, `longevity-${ts}.csv`);
}

async function runGenerator(opts) {
  if (!opts.dir) {
    console.error("[longevity] --dir is required in generator mode");
    process.exitCode = 1;
    return;
  }
  fs.mkdirSync(opts.dir, { recursive: true });

  const durationMs =
    opts.minutes != null ? opts.minutes * 60_000 : opts.hours * 3_600_000;
  const intervalMs =
    opts.intervalSec != null ? opts.intervalSec * 1000 : opts.intervalMin * 60_000;
  const sampleIntervalMs = opts.sampleIntervalSec * 1000;
  const sizes = parseSizesList(opts.sizes);
  const csvPath = opts.out || defaultOutPath();
  const manifestPath = opts.manifest || path.join(opts.dir, "manifest.json");
  const dbPath = opts.db || defaultDbPath();

  ensureCsvHeader(csvPath);

  const manifest = {
    generatedAt: new Date().toISOString(),
    seed: opts.seed,
    dir: path.resolve(opts.dir),
    sizes: sizes.map((s) => s.label),
    entries: [],
  };

  function flushManifest() {
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2));
  }

  function takeSample() {
    const rss = sampleRss(opts.processName);
    const db = getDbCounts(dbPath);
    const row = {
      ts: new Date().toISOString(),
      rss_kb: rss,
      files_local: db ? db.files_local : "",
      jobs_pending: db ? db.jobs_pending : "",
      jobs_uploading: db ? db.jobs_uploading : "",
      jobs_done: db ? db.jobs_done : "",
      jobs_failed: db ? db.jobs_failed : "",
    };
    appendCsvRow(csvPath, row);
    console.log(
      `[longevity] sample ${row.ts} rss_kb=${row.rss_kb ?? "?"} files_local=${row.files_local ?? "?"}`,
    );
    return row;
  }

  function generateOneFile(n) {
    const size = sizes[n % sizes.length];
    const fileName = `lt_${n}_${size.label}.bin`;
    const filePath = path.join(opts.dir, fileName);
    const sha256 = writeDeterministicFile(filePath, size.bytes, opts.seed, n);
    const entry = { ts: new Date().toISOString(), n, file: fileName, size: size.bytes, sha256 };
    manifest.entries.push(entry);
    flushManifest();
    console.log(`[longevity] generated ${fileName} (${size.bytes} bytes) sha256=${sha256.slice(0, 12)}...`);
    return entry;
  }

  let stopped = false;
  const onSigint = () => {
    console.log("\n[longevity] SIGINT received, flushing and stopping...");
    stopped = true;
  };
  process.on("SIGINT", onSigint);

  const start = Date.now();
  const end = start + durationMs;
  let n = 0;
  let nextGen = start;
  let nextSample = start;

  console.log(
    `[longevity] starting generator: dir=${opts.dir} durationMs=${durationMs} intervalMs=${intervalMs} sampleIntervalMs=${sampleIntervalMs}`,
  );

  while (!stopped) {
    const now = Date.now();
    if (now >= end) break;
    const target = Math.min(nextGen, nextSample, end);
    const wait = Math.max(0, target - now);
    if (wait > 0) await sleep(Math.min(wait, 1000));
    if (stopped) break;
    const now2 = Date.now();
    if (now2 >= nextSample) {
      takeSample();
      nextSample += sampleIntervalMs;
    }
    if (now2 >= nextGen && now2 < end) {
      generateOneFile(n);
      n++;
      nextGen += intervalMs;
    }
  }

  // Final sample + flush regardless of how the loop ended.
  const lastRow = takeSample();
  flushManifest();
  process.off("SIGINT", onSigint);

  const elapsedSec = ((Date.now() - start) / 1000).toFixed(1);
  console.log("=== Longevity generator summary ===");
  console.log(`Files generated: ${manifest.entries.length}`);
  console.log(`Elapsed: ${elapsedSec}s`);
  console.log(`Manifest: ${path.resolve(manifestPath)}`);
  console.log(`CSV: ${path.resolve(csvPath)}`);
  console.log(`Last RSS sample: ${lastRow.rss_kb ?? "n/a"} KB`);
}

// ---------------------------------------------------------------------------
// Report mode
// ---------------------------------------------------------------------------

export function median(nums) {
  if (nums.length === 0) return null;
  const sorted = [...nums].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid];
}

/** Parses the CSV produced by the generator into an array of row objects. */
export function parseCsv(content) {
  const lines = content.trim().split("\n").filter(Boolean);
  if (lines.length === 0) return [];
  const header = lines[0].split(",");
  return lines.slice(1).map((line) => {
    const values = line.split(",");
    const row = {};
    header.forEach((key, i) => {
      row[key] = values[i];
    });
    return row;
  });
}

/**
 * Computes RSS drift: median of the first-hour samples vs median of the
 * last-hour samples (relative to the run's own start/end, not wall-clock
 * hour boundaries — for a run shorter than 2h the windows overlap, which
 * is flagged in the result via `shortRun: true`).
 */
export function computeRssStats(rows) {
  const samples = rows
    .filter((r) => typeof r.rss_kb === "string" && r.rss_kb.trim() !== "")
    .map((r) => ({ ts: Date.parse(r.ts), rss: Number(r.rss_kb) }))
    .filter((r) => Number.isFinite(r.ts) && Number.isFinite(r.rss));
  if (samples.length === 0) return null;
  samples.sort((a, b) => a.ts - b.ts);
  const min = Math.min(...samples.map((s) => s.rss));
  const max = Math.max(...samples.map((s) => s.rss));
  const start = samples[0].ts;
  const end = samples[samples.length - 1].ts;
  const HOUR = 3_600_000;
  const firstHour = samples.filter((s) => s.ts <= start + HOUR);
  const lastHour = samples.filter((s) => s.ts >= end - HOUR);
  const medianFirst = median(firstHour.map((s) => s.rss));
  const medianLast = median(lastHour.map((s) => s.rss));
  const driftPct =
    medianFirst && medianFirst !== 0 ? (Math.abs(medianLast - medianFirst) / medianFirst) * 100 : null;
  return {
    sampleCount: samples.length,
    min,
    max,
    medianFirst,
    medianLast,
    driftPct,
    shortRun: end - start < 2 * HOUR,
  };
}

/** sha256 -> filenames map; returns entries whose sha256 collides (generator bug). */
export function findLocalDuplicates(entries) {
  const bySha = new Map();
  for (const e of entries) {
    if (!bySha.has(e.sha256)) bySha.set(e.sha256, []);
    bySha.get(e.sha256).push(e.file);
  }
  const duplicates = [];
  for (const [sha256, files] of bySha) {
    if (files.length > 1) duplicates.push({ sha256, files });
  }
  return duplicates;
}

/** Parses a "name sha256" per line remote listing file into [{ name, sha256 }]. */
export function parseRemoteList(content) {
  return content
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .map((line) => {
      const parts = line.split(/\s+/);
      return { name: parts[0], sha256: parts[1] || null };
    });
}

/**
 * Cross-checks the local manifest against one remote listing.
 * `missing` = generated locally but absent remotely (a lost job).
 * `extra` = present remotely but not in the local manifest.
 * `hashMismatch` = same name, different sha256.
 * `duplicatesRemote` = same sha256 uploaded more than once (RF-039 violation).
 */
export function crossCheckRemote(localEntries, remoteEntries) {
  const localByName = new Map(localEntries.map((e) => [e.file, e]));
  const remoteByName = new Map(remoteEntries.map((e) => [e.name, e]));

  const missing = [...localByName.keys()].filter((name) => !remoteByName.has(name));
  const extra = [...remoteByName.keys()].filter((name) => !localByName.has(name));
  const hashMismatch = [...localByName.entries()]
    .filter(([name]) => remoteByName.has(name))
    .filter(([name, local]) => remoteByName.get(name).sha256 && remoteByName.get(name).sha256 !== local.sha256)
    .map(([name]) => name);

  const bySha = new Map();
  for (const e of remoteEntries) {
    if (!e.sha256) continue;
    if (!bySha.has(e.sha256)) bySha.set(e.sha256, []);
    bySha.get(e.sha256).push(e.name);
  }
  const duplicatesRemote = [...bySha.entries()]
    .filter(([, names]) => names.length > 1)
    .map(([sha256, names]) => ({ sha256, names }));

  return { count: remoteEntries.length, missing, extra, hashMismatch, duplicatesRemote };
}

function fmtList(items, max = 10) {
  if (items.length === 0) return "none";
  const shown = items.slice(0, max).join(", ");
  return items.length > max ? `${shown}, ... (+${items.length - max} more)` : shown;
}

function buildReportMarkdown({ manifest, rss, localDuplicates, s3, drive, thresholds }) {
  const localCount = manifest.entries.length;
  const rnf001Status = !rss
    ? "SKIPPED (no RSS samples)"
    : rss.driftPct == null
      ? "SKIPPED (insufficient data to compute drift)"
      : rss.driftPct <= thresholds.rssDriftPct
        ? "PASS"
        : "FAIL";

  function remoteRow(label, result) {
    if (!result) return `| ${label} | SKIPPED (no listing provided) | - | - | - |`;
    const countMatch = result.count === localCount ? "yes" : `no (local=${localCount}, ${label}=${result.count})`;
    return `| ${label} | ${result.count} | ${countMatch} | ${fmtList(result.missing)} | ${result.duplicatesRemote.length} |`;
  }

  const rf039Parts = [];
  rf039Parts.push(localDuplicates.length === 0);
  if (s3) rf039Parts.push(s3.missing.length === 0 && s3.duplicatesRemote.length === 0);
  if (drive) rf039Parts.push(drive.missing.length === 0 && drive.duplicatesRemote.length === 0);
  const rf039Status = !s3 && !drive
    ? localDuplicates.length === 0
      ? "SKIPPED (no remote listing provided; local generator self-check PASS)"
      : "FAIL (local generator produced duplicate sha256 — test data invalid)"
    : rf039Parts.every(Boolean)
      ? "PASS"
      : "FAIL";

  const lines = [];
  lines.push("# Longevity Test Report");
  lines.push("");
  lines.push(`Generated: ${new Date().toISOString()}`);
  lines.push("");
  lines.push("## Summary");
  lines.push("");
  lines.push(`- Local files generated: **${localCount}**`);
  lines.push(`- Local sha256 duplicates (generator self-check): **${localDuplicates.length}**`);
  if (rss) {
    lines.push(
      `- RSS: min=${rss.min} KB, max=${rss.max} KB, first-hour median=${rss.medianFirst} KB, last-hour median=${rss.medianLast} KB, drift=${
        rss.driftPct == null ? "n/a" : rss.driftPct.toFixed(2) + "%"
      }${rss.shortRun ? " (warning: run < 2h, first/last hour windows overlap)" : ""}`,
    );
  } else {
    lines.push("- RSS: no samples available");
  }
  lines.push("");
  lines.push("## Remote cross-check");
  lines.push("");
  lines.push("| Destination | Count | Count matches local | Missing (lost jobs) | Duplicate sha256 |");
  lines.push("|---|---|---|---|---|");
  lines.push(remoteRow("S3", s3));
  lines.push(remoteRow("Drive", drive));
  lines.push("");
  lines.push("## PASS / FAIL");
  lines.push("");
  lines.push("| Requirement | Criterion | Result |");
  lines.push("|---|---|---|");
  lines.push(`| RNF-001 | RSS drift <= ${thresholds.rssDriftPct}% over the run | ${rnf001Status} |`);
  lines.push(`| RF-039 | Zero duplicate deliveries per destination (same sha256 twice) | ${rf039Status} |`);
  lines.push("");
  if (localDuplicates.length > 0) {
    lines.push("### Local duplicate sha256 (generator bug — investigate before trusting this run)");
    lines.push("");
    for (const d of localDuplicates) {
      lines.push(`- \`${d.sha256}\`: ${d.files.join(", ")}`);
    }
    lines.push("");
  }
  lines.push(
    "_Note: this report only asserts what it can actually measure. A SKIPPED row means the required input (RSS samples or a remote listing) was not provided — it is not a PASS._",
  );
  return lines.join("\n");
}

function runReport(opts) {
  if (!opts.manifest || !opts.csv) {
    console.error("[longevity] --mode report requires --manifest and --csv");
    process.exitCode = 1;
    return;
  }
  const manifest = JSON.parse(fs.readFileSync(opts.manifest, "utf8"));
  const csvRows = parseCsv(fs.readFileSync(opts.csv, "utf8"));
  const rss = computeRssStats(csvRows);
  const localDuplicates = findLocalDuplicates(manifest.entries);

  let s3 = null;
  let drive = null;
  if (opts.s3List) {
    const remote = parseRemoteList(fs.readFileSync(opts.s3List, "utf8"));
    s3 = crossCheckRemote(manifest.entries, remote);
  }
  if (opts.driveList) {
    const remote = parseRemoteList(fs.readFileSync(opts.driveList, "utf8"));
    drive = crossCheckRemote(manifest.entries, remote);
  }

  const thresholds = { rssDriftPct: 10 };
  const markdown = buildReportMarkdown({ manifest, rss, localDuplicates, s3, drive, thresholds });

  const reportOut = opts.reportOut || path.join(__dirname, "longevity-report.md");
  fs.writeFileSync(reportOut, markdown);
  console.log(markdown);
  console.log("");
  console.log(`[longevity] report written to ${path.resolve(reportOut)}`);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.mode === "report") {
    runReport(opts);
  } else {
    await runGenerator(opts);
  }
}

const isMain = process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url;
if (isMain) {
  main().catch((err) => {
    console.error("[longevity] fatal error:", err);
    process.exitCode = 1;
  });
}
