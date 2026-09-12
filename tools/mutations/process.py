"""Selected process-foundation mutations; every fault must compile before testing."""
CONFIG = {'copy': ['Cargo.toml', 'Cargo.lock', 'src', 'tests', 'examples'],
 'build': ['cargo', 'test', '--offline', '--locked', '--no-run'],
 'command': ['cargo', 'test', '--offline', '--locked'],
 'language': 'n2f-rs',
 'mutants': [('secret',
              'display_discloses',
              'src/shared/secret/value.rs',
              'impl fmt::Display for SecretString {\n'
              "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n"
              '        f.write_str("[REDACTED]")\n'
              '    }\n'
              '}\n',
              'impl fmt::Display for SecretString {\n'
              "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n"
              '        f.write_str(&self.value)\n'
              '    }\n'
              '}\n'),
             ('secret',
              'debug_discloses',
              'src/shared/secret/value.rs',
              'impl fmt::Debug for SecretString {\n'
              "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n"
              '        f.write_str("[REDACTED]")\n'
              '    }\n'
              '}\n',
              'impl fmt::Debug for SecretString {\n'
              "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n"
              '        f.write_str(&self.value)\n'
              '    }\n'
              '}\n'),
             ('env',
              'empty_becomes_absent',
              'src/shared/env/reader.rs',
              '    pub fn string(&mut self, k: &str, fallback: &str) -> String {\n'
              '        let Ok(raw) = self.read(k) else {\n'
              '            return String::new();\n'
              '        };\n'
              '        let p = raw.is_some();\n'
              '        let v = raw.unwrap_or_else(|| fallback.into());\n'
              '        self.record(k, &v, p, false);\n'
              '        v\n'
              '    }\n',
              '    pub fn string(&mut self, k: &str, fallback: &str) -> String {\n'
              '        let Ok(raw) = self.read(k) else {\n'
              '            return String::new();\n'
              '        };\n'
              '        let p = raw.is_some();\n'
              '        let v = raw.filter(|v| !v.is_empty()).unwrap_or_else(|| fallback.into());\n'
              '        self.record(k, &v, p, false);\n'
              '        v\n'
              '    }\n'),
             ('env',
              'manifest_discloses',
              'src/shared/env/reader.rs',
              'value: if s { "[REDACTED]" } else { v }.into()',
              'value: v.into()'),
             ('env',
              'false_becomes_true',
              'src/shared/env/reader.rs',
              'Some("false") => false',
              'Some("false") => true'),
             ('provenance',
              'retry_skips_ordinal',
              'src/shared/provenance/factory.rs',
              '.attempt\n            .checked_add(1)',
              '.attempt\n            .checked_add(0)'),
             ('provenance',
              'child_loses_depth',
              'src/shared/provenance/factory.rs',
              'd.checked_add(1)',
              'd.checked_add(0)'),
             ('provenance',
              'cause_without_correlation',
              'src/shared/provenance/incoming.rs',
              'Ok(_) if i.correlation.is_none()',
              'Ok(_) if false'),
             ('provenance',
              'collision_guard_removed',
              'src/shared/provenance/factory.rs',
              'if forbidden.contains(&scope_id) {',
              'if forbidden.contains(&scope_id) && false {'),
             ('logger',
              'exclusive_floor',
              'src/shared/logger/runtime.rs',
              'level < c.level',
              'level <= c.level'),
             ('logger',
              'noop_emits',
              'src/shared/logger/runtime.rs',
              'config.format == "none",',
              'false,'),
             ('logger',
              'ignores_no_color',
              'src/shared/logger/config.rs',
              'terminal && !no_color',
              'terminal'),
             ('logger',
              'queue_exceeds_capacity',
              'src/shared/logger/delivery.rs',
              'mpsc::sync_channel::<Vec<u8>>(capacity)',
              'mpsc::sync_channel::<Vec<u8>>(capacity + 1)'),
             ('logger',
              'cause_presence_inverted',
              'src/shared/logger/projection.rs',
              'error.source().is_some()',
              'error.source().is_none()'),
             ('root',
              'summary_loses_retry',
              'src/root/demo.rs',
              '("attempts", 2i64.into())',
              '("attempts", 1i64.into())')]}

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

# This file is intentionally self-contained: it only reads its own clone.
ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = Path(tempfile.mkdtemp(prefix="n2f-process-mutations-"))
WORK = EVIDENCE / "work"
WORK.mkdir()
ENV = os.environ.copy()
ENV.update(NO_COLOR="1", CI="true", CARGO_TERM_COLOR="never")
ENV["CARGO_TARGET_DIR"] = str(EVIDENCE / "target")
ENV.setdefault("GOCACHE", str(EVIDENCE / "go-cache"))

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

for relative in CONFIG["copy"]:
    source, target = ROOT / relative, WORK / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        shutil.copytree(source, target)
    else:
        shutil.copy2(source, target)
HASHES = {str(p.relative_to(WORK)): digest(p) for p in WORK.rglob("*") if p.is_file()}
if CONFIG["language"] == "n2f-nest":
    if not (ROOT / "node_modules").is_dir():
        raise SystemExit("Install this clone's locked dependencies first.")
    (WORK / "node_modules").symlink_to(ROOT / "node_modules", target_is_directory=True)

print(json.dumps({"evidence": str(EVIDENCE)}), flush=True)

def run(command, log):
    result = subprocess.run(command, cwd=WORK, env=ENV, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
    (EVIDENCE / log).write_text(result.stdout)
    return result

def cases(output):
    if CONFIG["language"] == "n2f-go":
        return re.findall(r"--- FAIL: (.+?) \(", output)
    if CONFIG["language"] == "n2f-rs":
        return re.findall(r"test (\w+) \.\.\. FAILED", output)
    if "AssertionError" not in output:
        return []
    return re.findall(r"^\s*FAIL\s+(.+)$", output, flags=re.MULTILINE)

build = run(CONFIG["build"], "baseline-build.txt")
baseline = run(CONFIG["command"], "baseline-test.txt")
if build.returncode or baseline.returncode:
    raise SystemExit("Baseline failed; inspect " + str(EVIDENCE))

results = []
for module, label, relative, old, new in CONFIG["mutants"]:
    path = WORK / relative
    original = path.read_bytes()
    text = original.decode()
    if text.count(old) != 1:
        raise SystemExit(f"{label}: mutation site count is {text.count(old)}, expected 1")
    test_exit, failures = None, []
    try:
        path.write_text(text.replace(old, new, 1))
        built = run(CONFIG["build"], label + "-build.txt")
        if built.returncode:
            outcome = "invalid"
        else:
            tested = run(CONFIG["command"], label + "-test.txt")
            test_exit, failures = tested.returncode, cases(tested.stdout)
            outcome = "survived" if test_exit == 0 else "killed" if failures else "harness_error"
    finally:
        path.write_bytes(original)
    result = dict(module=module, mutant=label, file=relative, before=old, after=new,
                  build_exit=built.returncode, test_exit=test_exit, outcome=outcome,
                  failing_cases=failures)
    results.append(result)
    print(json.dumps(result), flush=True)
    (EVIDENCE / "in-progress.json").write_text(json.dumps(results, indent=2) + "\n")

restored = run(CONFIG["command"], "restored-baseline-test.txt")
report = dict(language=CONFIG["language"], scope="hand-selected secret/env/provenance/logger/root faults",
              build_command=CONFIG["build"], test_command=CONFIG["command"],
              baseline_build_passed=build.returncode == 0, baseline_passed=baseline.returncode == 0,
              results=results, source_hashes=HASHES, restored_baseline_passed=restored.returncode == 0,
              workspace_unchanged=all(digest(ROOT / p) == sha for p, sha in HASHES.items()),
              copy_restored=all(digest(WORK / p) == sha for p, sha in HASHES.items()))
(EVIDENCE / "results.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({key: value for key, value in report.items() if key not in ("results", "source_hashes")}), flush=True)
if not (report["restored_baseline_passed"] and report["workspace_unchanged"] and report["copy_restored"]) or any(r["outcome"] != "killed" for r in results):
    raise SystemExit(1)
