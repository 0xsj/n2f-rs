# Selected clock mutations

**Origin:** 2026-09-11, contracts and public tests followed by implementation and
fault injection. The completed run caught 2/2 valid selected faults for this module.

| Fault | Failing public scenario |
| --- | --- |
| wall set resets elapsed | `c03_wall_correction` |
| advance loses elapsed | `c02_advance`, `c03_wall_correction`, `c04_refusal_is_atomic`, `c06_shared_fake` |

Every counted mutant compiled before its test run and failed a named runtime
expectation. Initial and restored baselines passed. Hashes confirmed the copied
source/test/config files were restored and their workspace originals were unchanged.
The runner changes temporary copies only; Nest reuses installed node_modules
through a symlink and invokes the compiler/test CLI directly.

Run from this clone with `python3 tools/mutations/foundations.py` and its normal
language toolchain on PATH. Rust's runner is offline and needs the locked packages
cached; Nest needs its installed dependencies. Raw logs and a combined report are
written to the temporary directory printed by the runner. The runner exits nonzero
for survivors, invalid mutants, setup failures or changed originals.

[Evidence](mutations.json) records the exact replacements, commands and hashes.
[Initial red run](initial-red.json) records the missing-API failure before bodies
existed. Those first runs stopped at compilation/import, so they were not runtime
assertion failures. These are same-author, contract-driven tests with reference
implementation visibility, not independent or blind tests.

This is a hand-selected fault check, not an exhaustive mutation score. It does not
establish global uniqueness, OS entropy failure recovery, timer behavior, other
operating systems, database constraints or frontend compatibility.
