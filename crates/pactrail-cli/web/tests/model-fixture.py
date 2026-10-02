"""Deterministic local provider for browser tests against the real Pactrail engine.
No external requests, no credentials, no assertions about real model ability.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json, time, threading
recover_lock=threading.Lock()
recover_seen=False
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args): pass
 def do_POST(self):
  request=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
  model=request.get('model','fixture-read');messages=request.get('messages',[])
  global recover_seen
  recover_failure=False
  if model=='fixture-recover':
   with recover_lock:
    recover_failure=not recover_seen;recover_seen=True
  if model=='fixture-fail' or recover_failure:
   self.send_response(400);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(json.dumps({'error':{'message':'Deliberate test provider failure.'}}).encode());return
  if model=='fixture-stop':time.sleep(25)
  if model=='fixture-slow':time.sleep(3)
  wrote=any(m.get('role')=='assistant' and any(c.get('function',{}).get('name')=='write_file' for c in m.get('tool_calls',[])) for m in messages)
  call=None
  ran_process=any(m.get('role')=='assistant' and any(c.get('function',{}).get('name')=='run_process' for c in m.get('tool_calls',[])) for m in messages)
  if model=='fixture-process' and not ran_process:
   time.sleep(2)
   call={'id':'process-1','type':'function','function':{'name':'run_process','arguments':json.dumps({'program':'python3','args':['-c','from pathlib import Path; Path("permission-proof.txt").write_text("approved")'],'timeout_seconds':5})}}
  if model in ('fixture-edit','fixture-verified') and not wrote:
   content='pub fn add(a: i32, b: i32) -> i32 { a + b }\n#[cfg(test)] mod tests { #[test] fn addition() { assert_eq!(super::add(2,3),5); } }\n' if model=='fixture-verified' else '# Test candidate\n'
   name='src/lib.rs' if model=='fixture-verified' else 'candidate.md'
   call={'id':'write-1','type':'function','function':{'name':'write_file','arguments':json.dumps({'path':name,'content':content})}}
  content='Created the isolated candidate.' if wrote else '# Workspace overview\n\nThis is a disposable browser-test workspace. No source files were changed.'
  message={'role':'assistant','content':'' if call else content}
  if call:message['tool_calls']=[call]
  response={'id':'fixture-turn','model':model,'choices':[{'index':0,'message':message,'finish_reason':'tool_calls' if call else 'stop'}],'usage':{'prompt_tokens':20,'completion_tokens':8,'total_tokens':28}}
  self.send_response(200)
  if request.get('stream'):
   self.send_header('Content-Type','text/event-stream');self.end_headers()
   delta={'role':'assistant'}
   if call:delta['tool_calls']=[{'index':0,**call}]
   else:delta['content']=content
   chunks=[{'id':'fixture-turn','choices':[{'index':0,'delta':delta,'finish_reason':None}]},{'id':'fixture-turn','choices':[{'index':0,'delta':{},'finish_reason':'tool_calls' if call else 'stop'}]},{'id':'fixture-turn','choices':[],'usage':response['usage']}]
   try:
    for chunk in chunks:self.wfile.write(('data: '+json.dumps(chunk)+'\n\n').encode())
    self.wfile.write(b'data: [DONE]\n\n')
   except BrokenPipeError:pass
  else:
   self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(json.dumps(response).encode())
ThreadingHTTPServer(('127.0.0.1',4190),Handler).serve_forever()
