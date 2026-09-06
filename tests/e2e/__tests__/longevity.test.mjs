// tests/e2e/__tests__/longevity.test.mjs
//
// Unit tests for the longevity harness's pure logic (generator determinism,
// report math). Run with `npm run test:e2e:unit` (node --test) — kept out
// of `npm test` (vitest) deliberately, see tests/e2e/README.md.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";

import {
  parseArgs,
  parseSize,
  parseSizesList,
  writeDeterministicFile,
  median,
  parseCsv,
  computeRssStats,
  findLocalDuplicates,
  parseRemoteList,
  crossCheckRemote,
} from "../longevity.mjs";

// ---------------------------------------------------------------------------
// parseArgs / size parsing
// ---------------------------------------------------------------------------

test("parseArgs applies defaults and reads flags", () => {
  const opts = parseArgs(["--dir", "/tmp/x", "--hours", "1", "--dry-run"]);
  assert.equal(opts.dir, "/tmp/x");
  assert.equal(opts.hours, 1);
  assert.equal(opts.dryRun, true);
  assert.equal(opts.mode, "generate");
  assert.equal(opts.seed, 42);
});

test("parseSize handles KB/MB/GB", () => {
  assert.equal(parseSize("16KB"), 16 * 1024);
  assert.equal(parseSize("4MB"), 4 * 1024 * 1024);
  assert.equal(parseSize("1GB"), 1024 * 1024 * 1024);
});

test("parseSize rejects garbage", () => {
  assert.throws(() => parseSize("banana"));
});

test("parseSizesList parses a cycling list", () => {
  const sizes = parseSizesList("16KB,512KB,4MB");
  assert.deepEqual(
    sizes.map((s) => s.label),
    ["16KB", "512KB", "4MB"],
  );
  assert.equal(sizes[0].bytes, 16 * 1024);
});

// ---------------------------------------------------------------------------
// Generator: distinct sha256 per file, exact sizes
// ---------------------------------------------------------------------------

test("writeDeterministicFile produces the exact requested size", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "longevity-test-"));
  const filePath = path.join(dir, "f0.bin");
  writeDeterministicFile(filePath, 12345, 42, 0);
  const stat = fs.statSync(filePath);
  assert.equal(stat.size, 12345);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("writeDeterministicFile is deterministic for the same (seed, n)", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "longevity-test-"));
  const p1 = path.join(dir, "a.bin");
  const p2 = path.join(dir, "b.bin");
  const sha1 = writeDeterministicFile(p1, 5000, 7, 3);
  const sha2 = writeDeterministicFile(p2, 5000, 7, 3);
  assert.equal(sha1, sha2);
  assert.equal(
    crypto.createHash("sha256").update(fs.readFileSync(p1)).digest("hex"),
    sha1,
  );
  fs.rmSync(dir, { recursive: true, force: true });
});

test("writeDeterministicFile produces distinct sha256 across file indices", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "longevity-test-"));
  const shas = new Set();
  for (let n = 0; n < 20; n++) {
    const filePath = path.join(dir, `f${n}.bin`);
    const sha = writeDeterministicFile(filePath, 2048, 42, n);
    shas.add(sha);
  }
  assert.equal(shas.size, 20, "expected 20 distinct sha256 values, generator collided");
  fs.rmSync(dir, { recursive: true, force: true });
});

test("writeDeterministicFile spans multiple chunks correctly (>1MB)", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "longevity-test-"));
  const filePath = path.join(dir, "big.bin");
  const size = 1024 * 1024 + 777; // > CHUNK_SIZE
  const sha = writeDeterministicFile(filePath, size, 1, 0);
  const stat = fs.statSync(filePath);
  assert.equal(stat.size, size);
  const actualSha = crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
  assert.equal(actualSha, sha);
  fs.rmSync(dir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// Report math: median / RSS drift
// ---------------------------------------------------------------------------

test("median handles odd and even length arrays", () => {
  assert.equal(median([1, 2, 3]), 2);
  assert.equal(median([1, 2, 3, 4]), 2.5);
  assert.equal(median([]), null);
});

test("parseCsv parses the generator's CSV shape", () => {
  const csv = [
    "ts,rss_kb,files_local,jobs_pending,jobs_uploading,jobs_done,jobs_failed",
    "2026-01-01T00:00:00.000Z,100000,10,2,1,7,0",
    "2026-01-01T01:00:00.000Z,105000,10,0,0,10,0",
  ].join("\n");
  const rows = parseCsv(csv);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].rss_kb, "100000");
  assert.equal(rows[1].jobs_done, "10");
});

test("computeRssStats computes drift within threshold (PASS case)", () => {
  const start = Date.parse("2026-01-01T00:00:00.000Z");
  const HOUR = 3_600_000;
  const rows = [
    { ts: new Date(start).toISOString(), rss_kb: "100000" },
    { ts: new Date(start + HOUR * 0.5).toISOString(), rss_kb: "101000" },
    { ts: new Date(start + HOUR * 70).toISOString(), rss_kb: "104000" },
    { ts: new Date(start + HOUR * 71.5).toISOString(), rss_kb: "105000" },
    { ts: new Date(start + HOUR * 72).toISOString(), rss_kb: "106000" },
  ];
  const stats = computeRssStats(rows);
  assert.ok(stats);
  assert.equal(stats.min, 100000);
  assert.equal(stats.max, 106000);
  assert.ok(stats.driftPct < 10, `expected drift < 10%, got ${stats.driftPct}`);
  assert.equal(stats.shortRun, false);
});

test("computeRssStats flags a FAIL-magnitude drift", () => {
  const start = Date.parse("2026-01-01T00:00:00.000Z");
  const HOUR = 3_600_000;
  const rows = [
    { ts: new Date(start).toISOString(), rss_kb: "100000" },
    { ts: new Date(start + HOUR * 0.5).toISOString(), rss_kb: "100000" },
    { ts: new Date(start + HOUR * 71.5).toISOString(), rss_kb: "150000" },
    { ts: new Date(start + HOUR * 72).toISOString(), rss_kb: "150000" },
  ];
  const stats = computeRssStats(rows);
  assert.ok(stats.driftPct > 10);
});

test("computeRssStats returns null when there are no numeric samples", () => {
  const rows = [{ ts: "2026-01-01T00:00:00.000Z", rss_kb: "" }];
  assert.equal(computeRssStats(rows), null);
});

// ---------------------------------------------------------------------------
// Report math: duplicates + remote cross-check
// ---------------------------------------------------------------------------

test("findLocalDuplicates finds sha256 collisions in the manifest", () => {
  const entries = [
    { file: "a.bin", sha256: "aaa" },
    { file: "b.bin", sha256: "bbb" },
    { file: "c.bin", sha256: "aaa" },
  ];
  const dups = findLocalDuplicates(entries);
  assert.equal(dups.length, 1);
  assert.equal(dups[0].sha256, "aaa");
  assert.deepEqual(dups[0].files.sort(), ["a.bin", "c.bin"]);
});

test("findLocalDuplicates returns empty for a clean manifest", () => {
  const entries = [
    { file: "a.bin", sha256: "aaa" },
    { file: "b.bin", sha256: "bbb" },
  ];
  assert.deepEqual(findLocalDuplicates(entries), []);
});

test("parseRemoteList parses 'name sha256' lines", () => {
  const list = parseRemoteList("a.bin aaa\nb.bin bbb\n");
  assert.deepEqual(list, [
    { name: "a.bin", sha256: "aaa" },
    { name: "b.bin", sha256: "bbb" },
  ]);
});

test("crossCheckRemote reports zero missing/extra/duplicates for a matching set", () => {
  const local = [
    { file: "a.bin", sha256: "aaa" },
    { file: "b.bin", sha256: "bbb" },
  ];
  const remote = parseRemoteList("a.bin aaa\nb.bin bbb\n");
  const result = crossCheckRemote(local, remote);
  assert.equal(result.count, 2);
  assert.deepEqual(result.missing, []);
  assert.deepEqual(result.extra, []);
  assert.deepEqual(result.hashMismatch, []);
  assert.deepEqual(result.duplicatesRemote, []);
});

test("crossCheckRemote detects a lost job (missing remotely)", () => {
  const local = [
    { file: "a.bin", sha256: "aaa" },
    { file: "b.bin", sha256: "bbb" },
  ];
  const remote = parseRemoteList("a.bin aaa\n");
  const result = crossCheckRemote(local, remote);
  assert.deepEqual(result.missing, ["b.bin"]);
});

test("crossCheckRemote detects a duplicate remote upload (RF-039 violation)", () => {
  const local = [{ file: "a.bin", sha256: "aaa" }];
  const remote = parseRemoteList("a.bin aaa\na-copy.bin aaa\n");
  const result = crossCheckRemote(local, remote);
  assert.equal(result.duplicatesRemote.length, 1);
  assert.equal(result.duplicatesRemote[0].sha256, "aaa");
});

test("crossCheckRemote detects a hash mismatch for the same name", () => {
  const local = [{ file: "a.bin", sha256: "aaa" }];
  const remote = parseRemoteList("a.bin zzz\n");
  const result = crossCheckRemote(local, remote);
  assert.deepEqual(result.hashMismatch, ["a.bin"]);
});
