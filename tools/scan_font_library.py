#!/usr/bin/env python3
"""Resumable read-only scan orchestration. Rust owns all font measurements.
Python standard library owns worker SQLite/checkpointing, not font parsing.
"""
from __future__ import annotations
import argparse, concurrent.futures, contextlib, hashlib, json, os, sqlite3, subprocess, sys, time
from datetime import datetime, timezone
from pathlib import Path

PROFILE = 'atlas-reference-v2'
GLYPHS = 'n,o,H,O,x,a,e,s,c,r,C,G,S,k,4,6,9,8,I,h,u,0,space'
SUPPORTED = {'.ttf','.otf','.ttc','.otc','.woff','.json','.ufo'}
SKIP_PARTS = {'.git','target','node_modules','.venv','__pycache__','do-not-index','archive','tests'}

def atomic_json(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + '.tmp')
    with temp.open('w', encoding='utf-8', newline='\n') as stream:
        json.dump(value, stream, ensure_ascii=False, sort_keys=True, allow_nan=False)
        stream.flush(); os.fsync(stream.fileno())
    os.replace(temp, path)

def digest(path: Path):
    h = hashlib.sha256()
    if path.is_dir():
        for child in sorted(path.rglob('*')):
            if child.is_symlink(): raise ValueError('source package contains a symlink')
            if child.is_file():
                relative = child.relative_to(path).as_posix().encode()
                data = child.read_bytes(); h.update(len(relative).to_bytes(8,'big'));h.update(relative)
                h.update(len(data).to_bytes(8,'big')); h.update(data)
    else:
        with path.open('rb') as stream:
            for chunk in iter(lambda:stream.read(1024*1024), b''): h.update(chunk)
    return h.hexdigest()

def discover(roots):
    paths, errors = set(), []
    for root in roots:
        root=Path(root)
        if not root.exists(): errors.append({'path':str(root),'error':'inventory root missing'});continue
        if root.is_file():paths.add(str(root.resolve()));continue
        for directory, dirs, files in os.walk(root, followlinks=False):
            dirs[:]=sorted(d for d in dirs if d.casefold() not in SKIP_PARTS and 'do-not-index' not in d.casefold() and not Path(directory,d).is_symlink())
            packages=[d for d in dirs if d.lower().endswith('.ufo')]
            for package in packages: paths.add(str(Path(directory,package).resolve()));dirs.remove(package)
            for name in sorted(files):
                p=Path(directory,name)
                if p.suffix.lower() in SUPPORTED|{'.woff2','.eot'} and not p.is_symlink():
                    if p.suffix.lower()=='.json':
                        # Only TypeFoundry documents, not arbitrary project data.
                        try:
                            if json.loads(p.read_text(encoding='utf-8-sig')).get('format')!='typefoundry.font':continue
                        except (ValueError,OSError,AttributeError):continue
                    paths.add(str(p.resolve()))
    return sorted(paths),errors

def scan_one(exe, path, source_hash, object_path, timeout):
    command=[str(exe),'analyze',path,'--glyphs',GLYPHS,'--profile','atlas']
    environment=os.environ.copy();environment['RAYON_NUM_THREADS']='1'
    try:
        completed=subprocess.run(command,capture_output=True,timeout=timeout,env=environment)
        if completed.returncode not in (0,1):raise ValueError(f'analyzer exited {completed.returncode}: '+completed.stderr.decode('utf-8','replace')[-1500:])
        value=json.loads(completed.stdout.decode('utf-8-sig'))
        if value.get('format')!='typefoundry.collection-study':raise ValueError('unexpected analyzer schema')
        if not value.get('fonts'):raise ValueError(json.dumps(value.get('errors',[]),ensure_ascii=False))
        if any(font['source']['sha256']!=source_hash for font in value['fonts']):raise ValueError('source changed during measurement')
        # Include only original measurements. Collection relationships are recomputed after deduplication.
        for font in value['fonts']:font['nearest']=[];font['outliers']=[]
        atomic_json(object_path,value)
        return 'measured',None,len(value['fonts'])
    except subprocess.TimeoutExpired:return 'timeout',f'analyzer exceeded {timeout}s; rerun with a longer timeout',0
    except (OSError,ValueError,KeyError) as error:return 'error',str(error),0

@contextlib.contextmanager
def worker_lock(store):
    store=Path(store);store.mkdir(parents=True,exist_ok=True)
    stream=(store/'worker.lock').open('a+b');stream.seek(0)
    if not stream.read(1):stream.write(b'0');stream.flush()
    stream.seek(0)
    try:
        if os.name=='nt':
            import msvcrt
            msvcrt.locking(stream.fileno(),msvcrt.LK_NBLCK,1)
        else:
            import fcntl
            fcntl.flock(stream.fileno(),fcntl.LOCK_EX|fcntl.LOCK_NB)
    except OSError:
        stream.close();raise RuntimeError('another scan is already using this worker store')
    try:yield
    finally:stream.close()

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--foundry',required=True,type=Path);parser.add_argument('--store',required=True,type=Path)
    parser.add_argument('--inventory',type=Path);parser.add_argument('--roots',nargs='*',default=[])
    parser.add_argument('--jobs',type=int,default=2);parser.add_argument('--timeout',type=int,default=120)
    parser.add_argument('--retry-errors',action='store_true');parser.add_argument('--atlas-out',type=Path)
    args=parser.parse_args()
    if not 1<=args.jobs<=8 or args.timeout<1:parser.error('jobs must be 1..8 and timeout positive')
    with worker_lock(args.store):return run_scan(args)

def run_scan(args):
    store=args.store.resolve();store.mkdir(parents=True,exist_ok=True)
    exe=args.foundry.resolve()
    if not exe.is_file():parser.error('foundry executable missing')
    inventory=json.loads(args.inventory.read_text(encoding='utf-8-sig')) if args.inventory else {'paths':[]}
    paths=inventory.get('paths',[]);discovered,inventory_errors=discover(args.roots)
    paths=sorted(set(paths+discovered))
    # Never allow a generated worker store to be scanned as a source.
    paths=[p for p in paths if not Path(p).is_relative_to(store) and 'do-not-index' not in p.casefold()]
    scan_key=hashlib.sha256((digest(exe)+PROFILE+GLYPHS).encode()).hexdigest()
    db=sqlite3.connect(store/'scan.sqlite')
    version=db.execute('PRAGMA user_version').fetchone()[0]
    if version not in (0,1):db.close();raise RuntimeError('unsupported worker database version; migration required')
    db.executescript(Path(__file__).with_name('migrations').joinpath('001_font_library.sql').read_text(encoding='utf-8-sig'))
    now=lambda:datetime.now(timezone.utc).isoformat()
    run=db.execute('INSERT INTO runs(started,status,inventory_count)VALUES(?,?,?)',(now(),'running',len(paths))).lastrowid;db.commit()
    atomic_json(store/'inventory.json',{'paths':paths,'roots':args.roots,'errors':inventory_errors,'profile':PROFILE,'scan_key':scan_key})
    objects={};completed=0;pending=[]
    for path in paths:
        p=Path(path)
        try:
            stat=p.stat()
            if p.suffix.lower() in {'.woff2','.eot'}:
                db.execute('INSERT OR REPLACE INTO sources VALUES(?,?,?,?,?,?,?,?)',(path,None,stat.st_size,stat.st_mtime_ns,'unsupported','WOFF2/EOT deliberately refused; use an existing desktop source',None,scan_key));completed+=1;continue
            source_hash=digest(p)  # content is authoritative even if size/mtime were preserved
            key=hashlib.sha256((scan_key+source_hash).encode()).hexdigest();object_path=store/'studies'/f'{key}.json'
            db.execute('INSERT OR REPLACE INTO sources VALUES(?,?,?,?,?,?,?,?)',(path,source_hash,stat.st_size,stat.st_mtime_ns,'pending',None,str(object_path),scan_key))
            objects.setdefault(key,{'hash':source_hash,'path':path,'object':object_path,'aliases':[]})['aliases'].append(path)
        except (OSError,ValueError) as error:
            db.execute('INSERT OR REPLACE INTO sources VALUES(?,?,?,?,?,?,?,?)',(path,None,None,None,'missing',str(error),None,scan_key));completed+=1
    db.commit()
    for key,item in objects.items():
        old=db.execute('SELECT status,error,faces FROM objects WHERE object_key=?',(key,)).fetchone()
        if old and ((old[0]=='measured' and item['object'].is_file()) or (old[0]!='measured' and not args.retry_errors)):
            db.execute('UPDATE sources SET status=?,error=? WHERE object_path=?',(old[0],old[1],str(item['object'])));completed+=len(item['aliases'])
        else:pending.append((key,item))
    db.commit()
    def progress():
        counts=dict(db.execute('SELECT status,count(*) FROM sources GROUP BY status'))
        value={'status':'running','run':run,'inventory_paths':len(paths),'completed_paths':completed,'unique_contents':len(objects),'counts':counts,'profile':PROFILE,'scan_key':scan_key}
        atomic_json(store/'progress.json',value);print(json.dumps(value),flush=True)
    progress()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        jobs={pool.submit(scan_one,exe,item['path'],item['hash'],item['object'],args.timeout):(key,item) for key,item in pending}
        for future in concurrent.futures.as_completed(jobs):
            key,item=jobs[future];status,error,faces=future.result()
            db.execute('INSERT OR REPLACE INTO objects VALUES(?,?,?,?,?,?)',(key,item['hash'],str(item['object']),status,error,faces))
            db.execute('UPDATE sources SET status=?,error=? WHERE object_path=?',(status,error,str(item['object'])))
            completed+=len(item['aliases']);db.execute('UPDATE runs SET completed=? WHERE id=?',(completed,run));db.commit()
            if completed%20==0 or status!='measured':progress()
    db.execute('UPDATE runs SET status=?,finished=?,completed=? WHERE id=?',('complete',now(),completed,run));db.commit()
    source_rows=[dict(zip(['path','sha256','status','error','object'],row)) for row in db.execute('SELECT path,sha256,status,error,object_path FROM sources ORDER BY path')]
    atomic_json(store/'sources.json',{'format':'typefoundry.source-inventory','version':1,'sources':source_rows,'inventory_errors':inventory_errors,'profile':PROFILE})
    progress();value=json.loads((store/'progress.json').read_text());value['status']='complete';atomic_json(store/'progress.json',value)
    print(json.dumps(value),flush=True)
    db.close()
    if args.atlas_out:
        return subprocess.run([str(exe),'atlas',str(store/'studies'),str(store/'sources.json'),'--out',str(args.atlas_out)]).returncode
    return 0
if __name__=='__main__':sys.exit(main())
