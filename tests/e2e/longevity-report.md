# Longevity Test Report

Generated: 2026-09-04T17:39:23.392Z

## Summary

- Local files generated: **6**
- Local sha256 duplicates (generator self-check): **0**
- RSS: no samples available

## Remote cross-check

| Destination | Count | Count matches local | Missing (lost jobs) | Duplicate sha256 |
|---|---|---|---|---|
| S3 | SKIPPED (no listing provided) | - | - | - |
| Drive | SKIPPED (no listing provided) | - | - | - |

## PASS / FAIL

| Requirement | Criterion | Result |
|---|---|---|
| RNF-001 | RSS drift <= 10% over the run | SKIPPED (no RSS samples) |
| RF-039 | Zero duplicate deliveries per destination (same sha256 twice) | SKIPPED (no remote listing provided; local generator self-check PASS) |

_Note: this report only asserts what it can actually measure. A SKIPPED row means the required input (RSS samples or a remote listing) was not provided — it is not a PASS._