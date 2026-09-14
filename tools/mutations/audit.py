#!/usr/bin/env python3
"""Run selected audit projection and per-consumer receipt mutations in isolated copies."""
import hashlib,json,os,pathlib,re,shutil,subprocess,tempfile

ROOT=pathlib.Path(__file__).resolve().parents[2]
LANG="rs"
EVIDENCE=pathlib.Path(tempfile.mkdtemp(prefix="n2f-audit-mutations-"))
WORK=EVIDENCE/"work"
shutil.copytree(ROOT,WORK,ignore=shutil.ignore_patterns(".git","node_modules","target","dist","notes",".env","coverage"))
NODE24=os.path.expanduser("~/.nvm/versions/node/v24.19.0/bin")
ENV={**os.environ,"PATH":NODE24+os.pathsep+os.environ.get("PATH",""),"CARGO_TARGET_DIR":"/private/tmp/n2f-audit-mutations-rs","CARGO_TERM_COLOR":"never","NO_COLOR":"1"}
MUTATIONS=[
    ("occurred_time_replaced", "src/domains/audit/mod.rs", "occurred_at_ms: wire.occurred_at_ms as i64,", "occurred_at_ms: recorded_at_ms,"),
    ("receipt_consumer_scope_ignored", "src/root/audit.rs", ".consume(&self.consumer, move |tx, event| {", '.consume("other", move |tx, event| {'),
    ("audit_action_projection_bypassed", "src/domains/audit/mod.rs", "action: wire.event_type,", 'action: "identity.fake.v1".into(),'),
]
COMMAND=["python3","tools/verify_audit.py"]
COMPILE_ERROR=re.compile(r"error\[E\d+\]|error TS\d+|error: could not compile|\[build failed\]|Transform failed|Failed to compile|cannot find module", re.I)

originals={};hashes={}
for _,file,_,_ in MUTATIONS:
    path=WORK/file
    if file not in originals:
        originals[file]=path.read_text();hashes[file]=hashlib.sha256(originals[file].encode()).hexdigest()

def run(name):
    env={**ENV,"N2F_CHECK_TMP":str(EVIDENCE/"checks"/name)}
    result=subprocess.run(COMMAND,cwd=WORK,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=900)
    (EVIDENCE/(name+".log")).write_text(result.stdout)
    return result

baseline=run("baseline")
assert baseline.returncode==0,("baseline failed",str(EVIDENCE))
results=[]
for name,file,old,new in MUTATIONS:
    path=WORK/file;original=originals[file]
    assert original.count(old)==1,(name,file,old)
    path.write_text(original.replace(old,new,1))
    try:
        result=run(name)
    finally:
        path.write_text(original)
    compiled=not COMPILE_ERROR.search(result.stdout)
    caught=result.returncode!=0 and compiled and ("AssertionError" in result.stdout or "Traceback" in result.stdout)
    results.append({"name":name,"file":file,"caught":caught,"compiled":compiled,"source_sha256":hashes[file]})
restored=run("restored")
assert restored.returncode==0,("restored baseline failed",str(EVIDENCE))
for file,digest in hashes.items():
    assert hashlib.sha256((ROOT/file).read_bytes()).hexdigest()==digest,("original source changed",file)
report={"language":LANG,"caught":sum(r["caught"] for r in results),"selected":len(results),"exhaustive":False,"evidence":str(EVIDENCE),"cases":results}
(EVIDENCE/"audit-mutation-evidence.json").write_text(json.dumps(report,indent=2))
print(json.dumps(report,indent=2))
assert all(r["caught"] for r in results)
