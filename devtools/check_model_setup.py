"""Real-binary onboarding regression tests; local fixture, dummy keys, no inference costs.
Requires the development terminal requirements. Does not change user configuration.
"""
import json, os, pathlib, subprocess, tempfile, threading, time, tomllib
from http.server import ThreadingHTTPServer
import pexpect

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(os.environ.get('PACTRAIL_CLI_TEST_BINARY', ROOT/'target/debug/pactrail'))
namespace = {}
fixture = (ROOT/'crates/pactrail-cli/web/tests/model-fixture.py').read_text()
exec(compile(fixture.split("ThreadingHTTPServer(('127.0.0.1',4190)")[0], 'local-model-fixture', 'exec'), namespace)
auth = []
class Handler(namespace['Handler']):
    def do_GET(self):
        auth.append(self.headers.get('Authorization'))
        self.send_response(200); self.send_header('Content-Type','application/json'); self.end_headers()
        self.wfile.write(json.dumps({'data':[{'id':'fixture-read'},{'id':'fixture-edit'}]}).encode())
    def do_POST(self):
        auth.append(self.headers.get('Authorization'))
        super().do_POST()
server = ThreadingHTTPServer(('127.0.0.1',0), Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}/v1'
key = 'fixture-private-setup-key'
passed=[]
with tempfile.TemporaryDirectory(prefix='pactrail-setup-qa-') as directory:
    root=pathlib.Path(directory); config=root/'config'; workspace=root/'workspace'; workspace.mkdir()
    (workspace/'README.md').write_text('# Disposable setup fixture\n')
    env=dict(os.environ,PACTRAIL_CONFIG_DIR=str(config),TERM='dumb',NO_COLOR='1',PACTRAIL_PLAIN='1')
    for name in ['PACTRAIL_MODEL','PACTRAIL_BASE_URL','OPENAI_API_KEY','OPENROUTER_API_KEY','ANTHROPIC_API_KEY','GEMINI_API_KEY']:
        env.pop(name,None)
    def spawn(args=['setup']):
        return pexpect.spawn(str(BINARY),['-C',str(workspace),*args],env=env,encoding='utf-8',timeout=30)
    def begin(child):
        child.expect('Provider'); child.sendline('7'); child.expect('API base URL'); child.sendline(endpoint)
    child=spawn(); begin(child); child.expect('Paste API key'); time.sleep(.15); child.sendline(key)
    child.expect('Model ID or search'); assert key not in child.before
    child.sendline('read'); child.expect('Select number'); child.sendline('1')
    child.expect('Model configured: fixture-read'); child.expect(pexpect.EOF); child.close(); assert child.exitstatus==0
    settings=tomllib.loads((config/'settings.toml').read_text())
    assert settings['process_backend']=='disabled'
    assert key not in (config/'settings.toml').read_text()
    assert auth==['Bearer '+key]
    secret_files=[p for p in config.rglob('*.txt') if key in p.read_text()]
    if os.name!='nt':
        assert len(secret_files)==1 and secret_files[0].stat().st_mode & 0o777 == 0o600
        assert secret_files[0].parent.stat().st_mode & 0o777 == 0o700
    passed.append('hidden key, search selection, unchanged permissions and private storage')
    if os.name!='nt':
        child=spawn([]); child.expect('pactrail >'); child.sendline('Explain this fixture')
        child.expect('Workspace overview'); child.expect('pactrail >'); child.sendline('/quit'); child.expect(pexpect.EOF)
        assert auth[-1]=='Bearer '+key
        assert key not in (config/'history.txt').read_text()
        passed.append('restart reuses endpoint-bound key for real engine task without history leakage')
        before=len(auth)
        result=subprocess.run([str(BINARY),'-C',str(workspace),'run','Explain fixture','--provider','open-ai-compatible','--model','fixture-read','--base-url',endpoint+'/different','--api-key-env',settings['api_key_env']],env=env,capture_output=True,text=True)
        assert result.returncode!=0 and 'different endpoint' in result.stderr and len(auth)==before
        passed.append('endpoint mismatch refused before any credential-bearing request')
    # Browser defaults must share the selected model/reference, never the key value.
    import socket, urllib.request
    probe=socket.socket(); probe.bind(('127.0.0.1',0)); web_port=probe.getsockname()[1]; probe.close()
    web=subprocess.Popen([str(BINARY),'-C',str(workspace),'web','--port',str(web_port)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        for attempt in range(100):
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{web_port}/api/bootstrap',timeout=1) as response:
                    body=response.read().decode(); bootstrap=json.loads(body)
                break
            except OSError: time.sleep(.05)
        else: raise AssertionError('web bootstrap unavailable')
        assert bootstrap['defaults']['model']=='fixture-read'
        assert bootstrap['defaults']['api_key_env']==settings['api_key_env'] and key not in body
    finally:
        web.terminate(); web.wait(timeout=10)
    passed.append('browser bootstrap shares model defaults without exposing the saved key')
    # Cancel replacement after discovery: the previous stored credential survives.
    child=spawn(['setup','--replace-key']); begin(child); child.expect('Paste API key'); time.sleep(.15); child.sendline('fixture-cancelled-replacement')
    child.expect('Model ID or search'); child.sendline('/cancel'); child.expect(pexpect.EOF)
    assert any(key in p.read_text() for p in config.rglob('*.txt'))
    assert not any('fixture-cancelled-replacement' in p.read_text() for p in config.rglob('*.txt'))
    passed.append('cancelled key replacement preserves the stored key')
    child=spawn(['setup','--replace-key']); begin(child); child.expect('Paste API key'); time.sleep(.15)
    replacement='fixture-accepted-replacement'; child.sendline(replacement)
    child.expect('Model ID or search'); assert replacement not in child.before; child.sendline('fixture-read')
    child.expect('Model configured: fixture-read'); child.expect(pexpect.EOF)
    assert not any(key in p.read_text() for p in config.rglob('*.txt'))
    key=replacement
    assert any(key in p.read_text() for p in config.rglob('*.txt'))
    passed.append('explicit replacement saves only the new key')
    previous=(config/'settings.toml').read_bytes()
    child=spawn(); child.expect('Provider'); child.sendcontrol('c'); child.expect(pexpect.EOF)
    assert (config/'settings.toml').read_bytes()==previous
    passed.append('cancel preserves prior configuration')
    # A fresh config exercises secret-prompt cancellation, including echo restoration.
    fresh=root/'fresh'; env['PACTRAIL_CONFIG_DIR']=str(fresh)
    child=spawn(); begin(child); child.expect('Paste API key'); time.sleep(.15); child.sendcontrol('c'); child.expect(pexpect.EOF)
    assert not (fresh/'settings.toml').exists()
    passed.append('Ctrl-C cancels hidden input without saving a credential')
    child=spawn([]); child.expect('pactrail >'); child.sendline('Explain this fixture')
    child.expect('Provider'); child.sendline('/cancel'); child.expect('task remains available'); child.expect('pactrail >')
    child.sendline('/quit'); child.expect(pexpect.EOF)
    assert any('Explain this fixture' in p.read_text() for p in fresh.rglob('*last-task.txt'))
    passed.append('first task retained when automatic setup is cancelled')
    # Explicit .env consent; don't import unrelated environment or save if cancelled at model step.
    (workspace/'.env').write_text('OPENAI_API_KEY='+key+'\nUNRELATED_PRIVATE_VALUE=never-import-this\n')
    child=spawn(); begin(child); child.expect('Use it for'); child.sendline(''); child.expect('Model ID or search'); child.sendline('/cancel'); child.expect(pexpect.EOF)
    assert not (fresh/'settings.toml').exists() and not any(key in p.read_text() for p in fresh.rglob('*.txt'))
    passed.append('workspace .env requires consent and cancelled selection writes no secret')
    child=spawn(); begin(child); child.expect('Use it for'); child.sendline('n'); child.expect('Paste API key'); time.sleep(.15); child.sendline('')
    child.expect('No API authentication selected'); child.expect('Model ID or search'); child.sendline('fixture-read')
    child.expect('Model configured: fixture-read'); child.expect(pexpect.EOF)
    assert tomllib.loads((fresh/'settings.toml').read_text())['api_key_env']=='PACTRAIL_NO_KEY'
    assert auth[-1] is None
    passed.append('unauthenticated compatible endpoint needs no dummy API key')
    env['PACTRAIL_CONFIG_DIR']=str(config)
    subprocess.run([str(BINARY),'setup','--forget-key'],env=env,check=True,capture_output=True)
    assert not any(key in p.read_text() for p in config.rglob('*.txt'))
    assert (workspace/'.env').exists()
    passed.append('forget removes saved credential without touching workspace .env')
server.shutdown(); server.server_close()
print(json.dumps({'passed':passed},indent=2))
