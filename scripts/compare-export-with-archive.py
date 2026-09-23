"""Compare `uncad export` of this branch with the archived monolithic implementation.

Build both release binaries first: this branch's into target/release/uncad.exe, and the
tag archive/feat-0.3-readable's (in a worktree, its own target dir) copied to
target/ab/uncad-feat.exe. Runs both over every DWG/DXF under lib/libredwg/test/test-data
at --max-levels 1 and compares the manifest counts. Paths below are the ones used on
2026-09-23; adjust them for your checkout.
"""
import os, json, subprocess, shutil
from concurrent.futures import ThreadPoolExecutor
NEW=r"C:\data\uncad\target\release\uncad.exe"; OLD=r"C:\data\uncad\target\ab\uncad-feat.exe"
BASE=r"C:\data\uncad\lib\libredwg\test\test-data"; OUT=r"C:\data\uncad\target\corpus_cmp"
shutil.rmtree(OUT, ignore_errors=True); os.makedirs(OUT)
files=[os.path.join(dp,f) for dp,_,fs in os.walk(BASE) for f in fs if f.lower().endswith(('.dwg','.dxf'))]
KEYS=["entities","texts","texts_paper","dimensions","geometry","regions","block_instances","hidden","sheets","frames"]
def one(f):
    rel=os.path.relpath(f,BASE).replace("\\","_")
    res={}
    for side,exe in (("old",OLD),("new",NEW)):
        d=os.path.join(OUT,side,rel)
        try:
            r=subprocess.run([exe,"export",f,"-o",d,"--max-levels","1"],capture_output=True,timeout=120)
            rc=r.returncode
        except subprocess.TimeoutExpired: rc="timeout"
        c={}
        try:
            m=json.load(open(os.path.join(d,"manifest.json"),encoding="utf-8")); c={k:m.get("counts",{}).get(k) for k in KEYS}
        except Exception: pass
        res[side]=(rc,c)
    return rel,res
with ThreadPoolExecutor(10) as ex: results=list(ex.map(one, files))
json.dump(results, open(os.path.join(OUT,"results.json"),"w"), indent=0)
both=[r for r in results if r[1]["old"][0]==0 and r[1]["new"][0]==0]
only_old=[r[0] for r in results if r[1]["old"][0]==0 and r[1]["new"][0]!=0]
only_new=[r[0] for r in results if r[1]["new"][0]==0 and r[1]["old"][0]!=0]
diff={}
for rel,res in both:
    for k in KEYS:
        a=res["old"][1].get(k); b=res["new"][1].get(k)
        if a!=b: diff.setdefault(k,[]).append((rel,a,b))
print(f"files {len(files)}: both ok {len(both)}, only old ok {len(only_old)}, only new ok {len(only_new)}")
print("only old:", only_old[:10]); print("only new:", only_new[:10])
for k,v in diff.items(): print(f"{k}: {len(v)} files differ, e.g. {v[:4]}")
if not diff: print("all manifest counts equal on files both read")
