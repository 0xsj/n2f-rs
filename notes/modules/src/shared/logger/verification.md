# Verification: logger

2026-09-11. The common contract and public-surface tests preceded implementation.
The author had implementation visibility; these are specification-first tests,
not an independent suite. Later boundary tests are ordinary regression tests.

Initial red: Go/Rust-style missing-public-symbol compilation failure. The [captured tool result](evidence/initial-red.json)
retains exit/output information; truncation is marked and is not a full raw log.
Current module/root coverage: **10 named tests** (scenario groups
may cover several contract IDs). This count excludes other modules and real-process
scenarios. Compiler refusals are counted separately below.

## Selected mutations

5 selected faults built successfully and were caught by tests. No invalid
mutant or harness failure was counted as a catch. Initial and restored baselines
passed; temporary-copy restoration and workspace hashes matched at the run.
[Exact mutations and failing cases](evidence/mutations.json) accompany complete
per-mutant build/test logs in evidence/. This finite fault set is not a coverage
or correctness percentage.

| Fault | Failing case(s) |
| --- | --- |
| exclusive_floor | f02_f03_modes_and_identities |
| noop_emits | f02_f03_modes_and_identities |
| ignores_no_color | l06_color_and_controls |
| queue_exceeds_capacity | l08_queue_and_deadline |
| cause_presence_inverted | l04_l05_safe_projections |


A final shared regression exposed literal DEL/C1 controls and Unicode line
separators in console output. [Failing cases](evidence/unicode-controls-red.json)
preceded a formatter correction in all three builds. Console messages and field
text now escape those characters before adding owned ANSI formatting. The full
15-fault set and eleven process checks were rerun after that correction.
