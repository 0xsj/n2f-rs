# Verification: provenance

2026-09-11. The common contract and public-surface tests preceded implementation.
The author had implementation visibility; these are specification-first tests,
not an independent suite. Later boundary tests are ordinary regression tests.

Initial red: Go/Rust-style missing-public-symbol compilation failure. The [captured tool result](evidence/initial-red.json)
retains exit/output information; truncation is marked and is not a full raw log.
Current module/root coverage: **9 named tests** (scenario groups
may cover several contract IDs). This count excludes other modules and real-process
scenarios. Compiler refusals are counted separately below.

## Selected mutations

4 selected faults built successfully and were caught by tests. No invalid
mutant or harness failure was counted as a catch. Initial and restored baselines
passed; temporary-copy restoration and workspace hashes matched at the run.
[Exact mutations and failing cases](evidence/mutations.json) accompany complete
per-mutant build/test logs in evidence/. This finite fault set is not a coverage
or correctness percentage.

| Fault | Failing case(s) |
| --- | --- |
| retry_skips_ordinal | f02_f03_modes_and_identities |
| child_loses_depth | f02_f03_modes_and_identities |
| cause_without_correlation | p15_p16_p17_incoming |
| collision_guard_removed | p23_explicit_cause_cannot_hide_execution_collision |

