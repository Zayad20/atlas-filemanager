"""Collect the pinned Go modules and their own notices from the shipped rclone.

Uses Go's checksum-verified module downloads; never executes downloaded code.
The resulting source bundle and notice file supplement the Rust/web inventory.
"""
import concurrent.futures,hashlib,json,os,re,subprocess,zipfile
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'target/rclone-audit';OUT.mkdir(parents=True,exist_ok=True)
binary=ROOT/'target/extensions/cloud/rclone.exe'
info=subprocess.run(['go','version','-m',str(binary)],check=True,capture_output=True,text=True).stdout
modules=[]
for line in info.splitlines():
    parts=line.strip().split('\t')
    if parts[0] in {'mod','dep'}:
        modules.append({'path':parts[1],'version':parts[2],'sum':parts[3] if len(parts)>3 else ''})
    elif parts[0]=='=>':
        modules[-1]['replacement']={'path':parts[1],'version':parts[2],'sum':parts[3] if len(parts)>3 else ''}

def fetch(item):
    actual=item.get('replacement',item)
    command=['go','mod','download','-json',actual['path']+'@'+actual['version']]
    r=subprocess.run(command,cwd=OUT,capture_output=True,text=True,env={**os.environ,'GOMAXPROCS':'2'},timeout=240)
    if r.returncode:raise RuntimeError('Go download failed for '+actual['path']+'; '+r.stderr[-250:])
    data=json.loads(r.stdout)
    if actual['sum'] and data.get('Sum')!=actual['sum']:raise ValueError('Module sum mismatch: '+actual['path'])
    source=Path(data['Dir']);notices=[]
    for p in sorted(source.rglob('*')):
        if p.is_file() and re.match(r'^(licen[sc]e|copying|copyright|notice|unlicense)([._-]|$)',p.name,re.I):
            relative=p.relative_to(source).as_posix()
            content=p.read_bytes()
            notices.append({'path':relative,'sha256':hashlib.sha256(content).hexdigest(),'text':content.decode('utf-8-sig',errors='replace')})
    if not notices:raise ValueError('Missing own notice: '+actual['path'])
    return {**item,'verifiedSum':data['Sum'],'zip':data['Zip'],'zipSha256':hashlib.sha256(Path(data['Zip']).read_bytes()).hexdigest(),'notices':notices}

with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
    results=list(pool.map(fetch,modules))
results.sort(key=lambda i:(i['path'],i['version']))
go_license=ROOT/'legal/upstream/go1.26.5-LICENSE.txt'
chunks=['rclone v1.74.4 — additional Go dependency notices\n\nThese are the modules recorded in the shipped Windows executable. Original\nmodule notices are preserved below. Dependencies retain their own licenses.\nSource and the verification manifest are available without charge at:\nhttps://github.com/neondeex/atlas-filemanager\n\nGo standard library, toolchain go1.26.5:\nhttps://github.com/golang/go/blob/go1.26.5/LICENSE\nhttps://go.dev/dl/\n\n'+go_license.read_text(encoding='utf-8')]
for item in results:
    chunks.append('\n'+'='*76+'\n'+item['path']+' '+item['version']+'\nChecksum: '+item['verifiedSum']+'\n')
    for notice in item['notices']:chunks.append('\n--- '+notice['path']+' ---\n'+notice['text'].strip()+'\n')
notice_path=ROOT/'packaging/windows/msix/RCLONE-THIRD-PARTY-NOTICES.txt'
notice_path.write_text('\n'.join(chunks),encoding='utf-8',newline='\n')
manifest={'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'goToolchain':'go1.26.5','modules':[{k:v for k,v in i.items() if k!='zip'} for i in results]}
(OUT/'modules.json').write_text(json.dumps(manifest,indent=2)+'\n',encoding='utf-8')
groups=[[]];size=0
for index,item in enumerate(results):
    length=Path(item['zip']).stat().st_size
    if size+length>200*1024*1024 and groups[-1]:groups.append([]);size=0
    groups[-1].append((index,item));size+=length
archives=[]
for part,group in enumerate(groups,1):
  archive=ROOT/f'target/source/atlas-0.1.1-tool-sources.part{part}.zip'
  with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED) as z:
    z.write(notice_path,'packaging/windows/msix/RCLONE-THIRD-PARTY-NOTICES.txt')
    z.write(OUT/'modules.json','RCLONE-MODULES.json')
    z.write(Path(__file__),'scripts/audit-rclone-notices.py')
    z.write(go_license,'legal/upstream/go1.26.5-LICENSE.txt')
    for index,item in group:z.write(item['zip'],f'go-module-sources/{index:03d}.zip')
    z.writestr('TOOL-SOURCES-README.txt','''These are the original checksum-verified Go module source archives for the
rclone v1.74.4 Windows executable shipped alongside Atlas 0.1.1. Their notices
are preserved in RCLONE-THIRD-PARTY-NOTICES.txt and RCLONE-MODULES.json maps
every original module, version, Go checksum and ZIP SHA-256 to its source.
Extract ALL tool-sources.partN.zip archives into the same directory. Module
ZIPs use the zero-based module index in RCLONE-MODULES.json, sorted by path and
version. The adjacent tool-sources.json lists every part and its SHA-256.
Go standard-library source is supplied separately from go.dev/dl (go1.26.5).
The independent rclone command is not relicensed as Atlas. Each module retains
its original license. No downloaded module code was executed by the collector.

For MSIX packaging, extract this over the Atlas source directory. After running
the supplied build-windows-msix.ps1, copy packaging/windows/msix/RCLONE-THIRD-PARTY-NOTICES.txt
into target/msix/staging, then repack that staging directory with the Windows
SDK MakeAppx.exe pack /o /d target/msix/staging /p target/msix/Atlas_0.1.1.0_x64.msix.
This adds notices only and does not change the compiled Atlas executables.
''')
  archives.append({'name':archive.name,'bytes':archive.stat().st_size,'sha256':hashlib.sha256(archive.read_bytes()).hexdigest(),'moduleIndices':[index for index,item in group]})
receipt={'version':'0.1.1','archives':archives,'modules':len(results),'notices':sum(len(i['notices']) for i in results),'noticeSha256':hashlib.sha256(notice_path.read_bytes()).hexdigest()}
(ROOT/'target/source/atlas-0.1.1-tool-sources.json').write_text(json.dumps(receipt,indent=2)+'\n')
print(json.dumps(receipt))
