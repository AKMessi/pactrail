#!/usr/bin/env python3
"""Reproducible controlled comparison. Standard library; never publishes credentials."""
import argparse, datetime, difflib, hashlib, http.server
import json, os, pathlib, secrets, signal, subprocess, sys, tempfile, threading, time
import urllib.error, urllib.request
from cases import CASES
ROOT=pathlib.Path(__file__).resolve().parent
MODEL='stealth/space-bunny-alpha'
ARMS=['pactrail-baseline','pactrail-audit','opencode','mini']
SUFFIX=' Work offline in this workspace only. Do not inspect history, remotes, other directories or benchmark graders. Do not install dependencies. Preserve the APIs and fix production files only. You may add temporary reproduction scripts but remove them before finishing. Run relevant checks and report any unverified behavior.'

def snapshot(path, names):
    return {name: digest(path/name) if (path/name).is_file() else None for name in names}

def digest(path): return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()
def write(path,data):
    path=pathlib.Path(path);path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(data,indent=2,ensure_ascii=False)+'\n')
def setup(path,case,gold=False):
    path.mkdir(parents=True)
    for name,body in (case['gold'] if gold else case['files']).items(): (path/name).write_text(body)
    (path/'README.md').write_text('# Controlled repository\nStandard-library Python modules. Run checks with python3.\n')
    for args in [['init','-q'],['add','.'],['-c','user.name=Benchmark','-c','user.email=benchmark@example.invalid','commit','-qm','Synthetic baseline']]:
        subprocess.run(['git','-C',str(path),*args],check=True,capture_output=True)
def grade(path,case):
    # Grader is external, passed via stdin, and never written into the agent workspace.
    sentinel='GRADER_COMPLETED_'+secrets.token_hex(24)
    script='import sys\nsys.path.insert(0,sys.argv[1])\n'+case['grader']+'\nprint('+repr(sentinel)+')\n'
    try:
        result=subprocess.run([sys.executable,'-B','-',str(path)],input=script,text=True,capture_output=True,timeout=15,cwd=path)
    except subprocess.TimeoutExpired:
        return {'passed':False,'stdout':'','stderr':'External grader exceeded 15 seconds.'}
    return {'passed':result.returncode==0 and sentinel in result.stdout.splitlines(),
            'stdout':result.stdout.replace(sentinel,'GRADER_COMPLETED'),'stderr':result.stderr}
def versions(args):
    result={}
    for name,path in [('baseline',args.baseline),('pactrail',args.pactrail),('opencode',args.opencode)]:
        result[name]={'sha256':digest(path),'version':subprocess.check_output([path,'--version'],text=True).strip()}
    result['mini']={'version':subprocess.check_output([args.mini_python,'-c','import importlib.metadata;print(importlib.metadata.version("mini-swe-agent"))'],text=True).strip()}
    return result

def freeze(args):
    if args.output.exists(): raise SystemExit('Refusing to overwrite an existing experiment directory.')
    validation=[]
    with tempfile.TemporaryDirectory(prefix='pactrail-grader-validation-') as tmp:
        for case in CASES:
            broken=pathlib.Path(tmp)/(case['id']+'-broken');gold=pathlib.Path(tmp)/(case['id']+'-gold')
            setup(broken,case);setup(gold,case,True)
            a=grade(broken,case);b=grade(gold,case)
            if a['passed'] or not b['passed']: raise RuntimeError('Invalid grader: '+case['id']+str(b))
            validation.append({'case':case['id'],'baseline':a,'gold':b})
    if args.metadata_file:
        model_metadata=next(m for m in json.loads(args.metadata_file.read_text()) if m['id']==MODEL)
        metadata_provenance={'source':'Captured OpenRouter API metadata; pricing is rechecked live before each scored trial.',
                             'captured_at':datetime.datetime.fromtimestamp(args.metadata_file.stat().st_mtime,datetime.timezone.utc).isoformat()}
    else:
        model_metadata=next(m for m in json.load(urllib.request.urlopen('https://openrouter.ai/api/v1/models',timeout=30))['data'] if m['id']==MODEL)
        metadata_provenance={'source':'https://openrouter.ai/api/v1/models','captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    if any(float(v)!=0 for v in model_metadata['pricing'].values()): raise RuntimeError('Model is not free; refusing experiment.')
    order=[]
    for repetition in range(args.repetitions):
        for index,case in enumerate(CASES):
            shift=(index+repetition)%len(ARMS)
            for arm in ARMS[shift:]+ARMS[:shift]: order.append({'case':case['id'],'arm':arm,'repetition':repetition+1})
    protocol={'schema_version':1,'frozen_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
       'description':'Exploratory controlled completeness suite; not SWE-bench and not evidence of universal superiority.',
       'base_commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
       'implementation_commit':args.implementation_commit,
       'implementation_sources':{str(p.relative_to(ROOT.parents[1])):digest(p) for p in [
           ROOT.parents[1]/'crates/pactrail-engine/src/engine.rs', ROOT.parents[1]/'crates/pactrail-engine/src/completion.rs',
           ROOT.parents[1]/'crates/pactrail-cli/src/cli.rs', ROOT.parents[1]/'crates/pactrail-cli/src/commands.rs']},
       'files':{name:digest(ROOT/name) for name in ['cases.py','run.py','mini_runner.py','summarize.py','test_benchmark.py']},
       'versions':versions(args),'model':MODEL,'metadata':model_metadata,'metadata_provenance':metadata_provenance,
       'controls':{'temperature':0,'reasoning_effort':'low','max_output_tokens':8192,'max_requests':12,
         'max_request_message_bytes':131072,'max_trial_seconds':300,'retries':'No trial retries or replacement samples; every HTTP attempt counts toward 12.',
         'permissions':'Trusted local command execution for all; sanitized environments; no root API key in agent processes.',
         'cost':'Only the exact currently-free model; no paid fallback. Zero API dollars cannot establish monetary savings.',
         'grading':'External behavioral assertions, forbidden production-path changes, and clean harness exit; no grader feedback during trials.'},
       'order':order,'grader_validation':validation}
    write(args.output/'protocol.json',protocol)
    print('Frozen',len(order),'trials; all baseline graders failed and all gold graders passed.',flush=True)

class Proxy:
    def __init__(self,key,folder,metadata):
        self.key=key;self.folder=folder;self.metadata=metadata;self.token=secrets.token_hex(24)
        self.records=[];self.lock=threading.Lock();self.active=True
        proxy=self
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self,*unused): pass
            def send_json(self,status,data):
                raw=json.dumps(data).encode();self.send_response(status);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
            def do_GET(self):
                if self.path.rstrip('/')=='/v1/models': self.send_json(200,{'data':[{'id':'space-bunny-alpha','object':'model'}]})
                else:self.send_json(404,{'error':'Unknown endpoint'})
            def do_POST(self):
                if self.headers.get('Authorization')!='Bearer '+proxy.token: self.send_json(401,{'error':'Unauthorized'});return
                if self.path not in ['/v1/chat/completions','/chat/completions']:self.send_json(404,{'error':'Unknown endpoint'});return
                length=int(self.headers.get('Content-Length','0'))
                if not 0<length<=1024*1024:self.send_json(413,{'error':'Request bound'});return
                body=json.loads(self.rfile.read(length));stream=bool(body.get('stream'))
                with proxy.lock:
                    if not proxy.active or len(proxy.records)>=12:self.send_json(429,{'error':'Frozen trial request allowance exhausted'});return
                    record={'attempt':len(proxy.records)+1,'started':time.time(),'requested_model':body.get('model'),'requested_max_tokens':body.get('max_tokens'),'usage':None}
                    proxy.records.append(record)
                if len(json.dumps(body.get('messages',[])).encode())>131072:
                    record['status']=413;proxy.save();self.send_json(413,{'error':'Frozen input byte allowance exceeded'});return
                # Normalize common API controls for every arm and retain requested controls.
                for name in ['thinking','reasoning_effort','max_completion_tokens','stream_options']:body.pop(name,None)
                body.update(model=MODEL,temperature=0,max_tokens=8192,reasoning={'effort':'low'},stream=False)
                request=urllib.request.Request('https://openrouter.ai/api/v1/chat/completions',data=json.dumps(body).encode(),headers={'Authorization':'Bearer '+proxy.key,'Content-Type':'application/json'})
                try:
                    with urllib.request.urlopen(request,timeout=120) as response:data=json.load(response)
                    record.update(status=200,usage=data.get('usage'),provider_id=data.get('id'),elapsed_seconds=time.time()-record['started'])
                    proxy.save()
                    if not stream:self.send_json(200,data);return
                    self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
                    choice=data['choices'][0];message=choice['message'];delta={k:v for k,v in message.items() if k in ['role','content','reasoning','reasoning_content','tool_calls'] and v is not None}
                    for i,call in enumerate(delta.get('tool_calls',[])):call['index']=i
                    for part in [{'id':data.get('id'),'object':'chat.completion.chunk','choices':[{'index':0,'delta':delta,'finish_reason':None}]},
                                 {'id':data.get('id'),'object':'chat.completion.chunk','choices':[{'index':0,'delta':{},'finish_reason':choice.get('finish_reason')}],'usage':data.get('usage')}]:
                        self.wfile.write(('data: '+json.dumps(part)+'\n\n').encode())
                    self.wfile.write(b'data: [DONE]\n\n');self.wfile.flush()
                except urllib.error.HTTPError as error:
                    record.update(status=error.code,elapsed_seconds=time.time()-record['started']);proxy.save();self.send_json(error.code,{'error':{'message':'Upstream HTTP '+str(error.code)}})
                except Exception as error:
                    record.update(status=502,error_type=type(error).__name__,elapsed_seconds=time.time()-record['started']);proxy.save()
                    try:self.send_json(502,{'error':{'message':'Upstream request failed: '+type(error).__name__}})
                    except (BrokenPipeError,ConnectionResetError):pass
        self.server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.server.daemon_threads=False
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True);self.thread.start()
        self.url='http://127.0.0.1:'+str(self.server.server_port)
    def save(self):
        with self.lock: write(self.folder/'requests.json',self.records)
    def close(self):self.active=False;self.server.shutdown();self.server.server_close();self.save()

def _trial(args,spec,case,metadata,key):
    name=f"r{spec['repetition']}-{case['id']}-{spec['arm']}";folder=args.output/'raw'/name
    if folder.exists():raise RuntimeError('Trial already exists; no retries allowed: '+name)
    folder.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix='pactrail-scored-') as tmp:
        base=pathlib.Path(tmp);workspace=base/'workspace';home=base/'home';home.mkdir();setup(workspace,case)
        prompt=case['prompt']+SUFFIX;promptfile=base/'task.txt';promptfile.write_text(prompt)
        proxy=Proxy(key,folder,metadata)
        try:
            # Deliberately exclude provider credentials and the repository root .env.
            env={'PATH':os.environ['PATH'],'HOME':str(home),'LANG':'C.UTF-8','TERM':'dumb','NO_COLOR':'1','PYTHONDONTWRITEBYTECODE':'1',
                 'XDG_CONFIG_HOME':str(home/'config'),'XDG_DATA_HOME':str(home/'data'),'XDG_STATE_HOME':str(home/'state'),
                 'BENCHMARK_PROXY_URL':proxy.url,'BENCHMARK_PROXY_KEY':proxy.token,'LITELLM_LOCAL_MODEL_COST_MAP':'True','DO_NOT_TRACK':'1'}
            arm=spec['arm'];config=base/'opencode.json'
            if arm.startswith('pactrail'):
                exe=args.baseline if arm=='pactrail-baseline' else args.pactrail
                command=[exe,'-C',str(workspace),'run',prompt,'--provider','open-ai-compatible','--model',MODEL,'--base-url',proxy.url+'/v1','--api-key-env','BENCHMARK_PROXY_KEY','--allow-process','--process-backend','native','--process-approval','allow-run','--max-turns','12','--context-tokens','32768','--max-output-tokens','8192','--output','json','--input-price','0','--cached-input-price','0','--cache-creation-price','0','--output-price','0']
                if arm=='pactrail-audit':command.append('--completion-audit')
            elif arm=='opencode':
                write(config,{'$schema':'https://opencode.ai/config.json','autoupdate':False,'permission':'allow','provider':{'benchmark':{'npm':'@ai-sdk/openai-compatible','name':'Benchmark proxy','options':{'baseURL':proxy.url+'/v1','apiKey':'{env:BENCHMARK_PROXY_KEY}'},'models':{'space-bunny-alpha':{'name':'Space Bunny Alpha','limit':{'context':32768,'output':8192}}}}},'agent':{'build':{'steps':12}}})
                env.update(OPENCODE_CONFIG=str(config),OPENCODE_DISABLE_DEFAULT_PLUGINS='1',OPENCODE_DISABLE_SHARE='1',OPENCODE_DISABLE_AUTOUPDATE='1')
                command=[args.opencode,'run','--pure','--format','json','--model','benchmark/space-bunny-alpha',prompt]
            else:command=[args.mini_python,str(ROOT/'mini_runner.py'),str(promptfile),str(folder/'trajectory.json')]
            started=time.monotonic();timed_out=False
            with (folder/'stdout.txt').open('w') as out,(folder/'stderr.txt').open('w') as err:
                process=subprocess.Popen(command,cwd=workspace,env=env,stdout=out,stderr=err,start_new_session=True)
                try:code=process.wait(timeout=300)
                except subprocess.TimeoutExpired:
                    timed_out=True;os.killpg(process.pid,signal.SIGTERM)
                    try:code=process.wait(timeout=5)
                    except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);code=process.wait()
            seconds=time.monotonic()-started
            # Do not let background agent descendants observe the independent grader.
            try: os.killpg(process.pid,signal.SIGKILL)
            except ProcessLookupError: pass
            proxy.close();isolation=None;trace_verified=None;applied_match=None;candidate=workspace
            candidate_before=None
            if arm.startswith('pactrail'):
                candidates=list((workspace/'.pactrail/runs').glob('*/workspace'))
                if len(candidates)==1: candidate=candidates[0]
                candidate_before=snapshot(candidate,case['files'])
                manifests=list((workspace/'.pactrail/runs').glob('*/receipt.json'))
                if manifests:
                    receipt=json.loads(manifests[0].read_text());write(folder/'receipt.json',receipt)
                    run=manifests[0].parent
                    if (run/'trace.jsonl').exists(): (folder/'trace.jsonl').write_bytes((run/'trace.jsonl').read_bytes())
                    trace=subprocess.run([exe,'-C',str(workspace),'trace',receipt['run_id'],'--json'],env=env,capture_output=True,text=True);trace_verified=trace.returncode==0
                    isolation=all((workspace/name).is_file() and (workspace/name).read_bytes()==body.encode() for name,body in case['files'].items())
                    if receipt['outcome']=='ready_to_apply':
                        applied=subprocess.run([exe,'-C',str(workspace),'apply',receipt['run_id'],'--json'],env=env,capture_output=True,text=True)
                        (folder/'apply.json').write_text(applied.stdout);code=code if applied.returncode==0 else applied.returncode
                        applied_match=applied.returncode==0
                    else:code=code or 1
                else:code=code or 1
            behavioral=grade(candidate,case)
            candidate_match=(snapshot(workspace,case['files'])==candidate_before) if applied_match else None
            original={**case['files'],'README.md':'# Controlled repository\nStandard-library Python modules. Run checks with python3.\n'}
            actual={str(p.relative_to(candidate)):p.read_bytes() for p in candidate.rglob('*')
                    if p.is_file() and not any(part in {'.git','.pactrail','__pycache__'} for part in p.relative_to(candidate).parts)}
            changed=[name for name in set(original)|set(actual) if actual.get(name)!=original.get(name,'').encode()]
            forbidden=sorted(name for name in changed if name not in case['files'])
            patch=[]
            for name in sorted(changed):
                patch.extend(difflib.unified_diff(original.get(name,'').splitlines(True),
                             actual.get(name,b'').decode(errors='replace').splitlines(True),
                             fromfile='a/'+name,tofile='b/'+name))
            (folder/'candidate.patch').write_text(''.join(patch))
            result={**spec,'returncode':code,'timed_out':timed_out,'agent_seconds':seconds,'functional':behavioral,
                    'forbidden_paths':forbidden,'strict_passed':behavioral['passed'] and not forbidden and code==0 and not timed_out,
                    'requests':len(proxy.records),'usage': [r['usage'] for r in proxy.records],
                    'provider_errors':[r.get('status') for r in proxy.records if r.get('status')!=200],
                    'source_isolation':isolation,'trace_verified':trace_verified,'apply_succeeded':applied_match,'applied_production_bytes_match':candidate_match}
            if arm.startswith('pactrail'):result['strict_passed'] &= isolation is True and trace_verified is True and applied_match is True and candidate_match is True
            write(folder/'result.json',result)
            print(name, 'PASS' if result['strict_passed'] else 'FAIL',f'{seconds:.1f}s',len(proxy.records),'requests',flush=True)
            return result
        finally:
            if proxy.active: proxy.close()

def trial(args,spec,case,metadata,key):
    name=f"r{spec['repetition']}-{case['id']}-{spec['arm']}"
    folder=args.output/'raw'/name
    if folder.exists(): raise RuntimeError('Refusing to replace an existing trial: '+name)
    try:
        return _trial(args,spec,case,metadata,key)
    except Exception as error:
        requests=json.loads((folder/'requests.json').read_text()) if (folder/'requests.json').is_file() else []
        result={**spec,'status':'harness_or_orchestration_error','error_type':type(error).__name__,
                'strict_passed':False,'functional':None,'requests':len(requests),
                'usage':[r.get('usage') for r in requests]}
        write(folder/'result.json',result)
        print(name,'ERROR',type(error).__name__,flush=True)
        return result

def run(args):
    protocol=json.loads((args.output/'protocol.json').read_text())
    if versions(args)!=protocol['versions']:raise RuntimeError('Frozen executable versions changed')
    for name,sha in protocol['files'].items():
        if digest(ROOT/name)!=sha:raise RuntimeError('Frozen source changed: '+name)
    for name,sha in protocol['implementation_sources'].items():
        if digest(ROOT.parents[1]/name)!=sha: raise RuntimeError('Frozen implementation source changed: '+name)
    # dotenv parsing occurs only in this orchestrator, never in an agent process.
    key=os.environ.get('OPENROUTER_API_KEY')
    if not key:
        for line in (ROOT.parents[1]/'.env').read_text().splitlines():
            line=line.strip().removeprefix('export ')
            if line.startswith('OPENROUTER_API_KEY='):key=line.partition('=')[2].strip().strip('\"\'');break
    if not key:raise RuntimeError('No OpenRouter credential found')
    cases={c['id']:c for c in CASES};results=[]
    for spec in protocol['order']:
        name=f"r{spec['repetition']}-{spec['case']}-{spec['arm']}"
        existing=args.output/'raw'/name
        if existing.exists():
            if (existing/'result.json').is_file():
                result=json.loads((existing/'result.json').read_text())
                if any(result.get(k)!=v for k,v in spec.items()): raise RuntimeError('Existing trial identity mismatch')
            else:
                records=json.loads((existing/'requests.json').read_text()) if (existing/'requests.json').is_file() else []
                result={**spec,'status':'interrupted_trial_retained_without_retry','strict_passed':False,
                        'functional':None,'requests':len(records),'usage':[r.get('usage') for r in records]}
                write(existing/'result.json',result)
            results.append(result);write(args.output/'results.json',results)
            continue
        # Refuse paid changes between trials as well as at preregistration.
        try:
            metadata=next(m for m in json.load(urllib.request.urlopen('https://openrouter.ai/api/v1/models',timeout=30))['data'] if m['id']==MODEL)
        except (OSError,urllib.error.URLError) as error:
            write(args.output/'execution-status.json',{'status':'blocked_before_trial','reason':type(error).__name__,
                    'completed_trials':len(results),'next_trial':spec,'scored_model_requests_sent':sum(r['requests'] for r in results)})
            raise RuntimeError('Live pricing/model availability could not be checked; no request sent for the next trial.') from error
        if any(float(v)!=0 for v in metadata['pricing'].values()):raise RuntimeError('Model ceased to be free; stop without replacing trials')
        results.append(trial(args,spec,cases[spec['case']],metadata,key));write(args.output/'results.json',results)
    print('Complete; every scored trial retained.',flush=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('action',choices=['freeze','run'])
    parser.add_argument('--output',type=pathlib.Path,required=True)
    parser.add_argument('--baseline',default='/tmp/pactrail-before-completion-audit')
    parser.add_argument('--pactrail',default=str(ROOT.parents[1]/'target/release/pactrail'))
    parser.add_argument('--opencode',default='/tmp/pactrail-opencode-pinned/package/bin/opencode')
    parser.add_argument('--mini-python',default='/tmp/pactrail-benchmark-venv/bin/python')
    parser.add_argument('--repetitions',type=int,default=2)
    parser.add_argument('--implementation-commit',help='Published implementation commit; source and binary hashes remain authoritative.')
    parser.add_argument('--metadata-file',type=pathlib.Path,help='Captured metadata for offline preregistration; scored runs still require live pricing checks.')
    args=parser.parse_args();args.output=args.output.resolve()
    if not 1 <= args.repetitions <= 10: parser.error('repetitions must be 1–10')
    freeze(args) if args.action=='freeze' else run(args)
