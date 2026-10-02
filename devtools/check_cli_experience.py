"""Development-only PTY checks. Requires pexpect and pyte outside the application.
Uses the real binary and local model fixture in disposable workspaces, no credentials.
Run: python devtools/check_cli_experience.py
"""
import json, os, pathlib, re, subprocess, sys, tempfile, time
import pexpect, pyte

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = pathlib.Path(os.environ.get('PACTRAIL_CLI_TEST_BINARY', str(ROOT / 'target/debug/pactrail')))
OUT = pathlib.Path('/tmp/pactrail-cli-qa')
OUT.mkdir(exist_ok=True)

class Capture:
    def __init__(self, cols, rows, name):
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.Stream(self.screen)
        self.log = (OUT / (name + '.ansi')).open('w')
        self.text = ''
    def write(self, text):
        self.stream.feed(text)
        self.text += text
        self.log.write(text)
    def flush(self): self.log.flush()
    def snapshot(self, name):
        (OUT / (name + '.txt')).write_text('\n'.join(self.screen.display))
        (OUT / (name + '.cells.json')).write_text(json.dumps([
            [{'text':self.screen.buffer[y][x].data, 'fg':self.screen.buffer[y][x].fg, 'bold':self.screen.buffer[y][x].bold} for x in range(self.screen.columns)]
            for y in range(self.screen.lines)
        ]))

class Terminal:
    def __init__(self, width=100, name='workflow', dumb=False, color=False, ascii=False, plain=False):
        self.root = pathlib.Path(tempfile.mkdtemp(prefix='pactrail-cli-qa-'))
        self.workspace = self.root / 'workspace'
        self.workspace.mkdir()
        (self.workspace / 'README.md').write_text('# Disposable CLI test\n')
        config = self.root / 'config'
        config.mkdir()
        (config / 'settings.toml').write_text('''schema = 1
provider = "open-ai-compatible"
model = "fixture-read"
base_url = "http://127.0.0.1:4190/v1"
api_key_env = "PACTRAIL_FIXTURE_KEY"
context_tokens = 32768
max_output_tokens = 1024
max_turns = 8
allow_process = false
''')
        editor = self.root / 'editor.py'
        editor.write_text('import pathlib, sys\np=pathlib.Path(sys.argv[1]); p.write_text(p.read_text()+"\\nEdited draft in external editor.")\n')
        self.env = dict(os.environ, PACTRAIL_CONFIG_DIR=str(config), NO_COLOR='1', PACTRAIL_NO_ANIMATION='1', TERM='dumb' if dumb else 'xterm-256color', EDITOR=f'{sys.executable} {editor}', PACTRAIL_FIXTURE_KEY='fixture-not-a-secret')
        self.color = color
        self.ascii = ascii or dumb or plain
        self.env['PACTRAIL_PLAIN'] = '1' if plain else '0'
        self.env['PACTRAIL_ASCII'] = '1' if ascii else '0'
        self.env['LC_ALL'] = 'C.UTF-8'
        if color: self.env.pop('NO_COLOR', None)
        for key in ['PACTRAIL_MODEL','PACTRAIL_BASE_URL','PAGER','VISUAL']:
            self.env.pop(key,None)
        self.child = pexpect.spawn(str(BINARY), ['--workspace', str(self.workspace)], env=self.env, encoding='utf-8', dimensions=(34,width), timeout=20)
        self.child.linesep = "\r"
        self.capture = Capture(width,34,name)
        self.child.logfile_read = self.capture
        self.expect('pactrail ❯')
        self.capture.snapshot(name + '-startup')
        assert any('Composer' in row and 'commands: none' in row for row in self.capture.screen.display), 'composer hid command permissions'
    def fork_session(self, name):
        other = object.__new__(Terminal)
        other.root, other.workspace, other.env, other.color = self.root, self.workspace, dict(self.env), self.color
        other.ascii = self.ascii
        other.capture = Capture(100,34,name)
        other.child = pexpect.spawn(str(BINARY), ['--workspace',str(self.workspace)], env=other.env, encoding='utf-8', dimensions=(34,100), timeout=20)
        other.child.linesep = "\r"; other.child.logfile_read = other.capture
        other.expect('pactrail ❯')
        return other
    def expect(self, pattern, timeout=20):
        if self.ascii:
            pattern = pattern.replace('pactrail ❯','pactrail >').replace('◇ ','<> ')
        if self.color and pattern == 'pactrail ❯':
            pattern = r'pactrail(?:\x1b\[[0-9;]*m)* ❯'
        if self.color and pattern.startswith('◇ '):
            pattern = r'◇(?:\x1b\[[0-9;]*m)* ' + re.escape(pattern[2:])
        deadline=time.monotonic()+timeout
        while True:
            index=self.child.expect([pattern, '\x1b\\[6n', pexpect.EOF], timeout=max(.1,deadline-time.monotonic()))
            if index==0: return
            if index==2: raise AssertionError('CLI exited unexpectedly: '+self.capture.text[-2000:])
            self.child.send(f'\x1b[{self.capture.screen.cursor.y+1};{self.capture.screen.cursor.x+1}R')
    def command(self, value, result=None):
        self.child.sendline(value)
        if result: self.expect(result)
        self.expect('pactrail ❯')
    def close(self):
        if self.child.isalive():
            self.child.sendline('/quit'); self.expect('Session closed')
        self.child.expect(pexpect.EOF)
        self.child.close()
        assert self.child.exitstatus==0
        if not self.color:
            assert not re.search(r'\x1b\[(?:3[0-7]|9[0-7]|38;[^m]*)m', self.capture.text), 'NO_COLOR emitted colored text'

provider=subprocess.Popen([sys.executable,str(ROOT/'crates/pactrail-cli/web/tests/model-fixture.py')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
terminals=[]
try:
    time.sleep(.3)
    assert provider.poll() is None, 'local model fixture could not bind 4190'
    t=Terminal();terminals.append(t)
    t.command('continue','No task to continue')
    # Bracketed multiline paste stays in the composer until explicit Enter.
    t.child.send('\x1b[200~Pasted task.\nDo not dispatch automatically.\x1b[201~')
    t.expect('Do not dispatch automatically.')
    assert not list((t.workspace/'.pactrail/runs').glob('*/run.json'))
    t.child.send('\x13');t.expect('Workspace draft saved');t.expect('pactrail ❯')
    t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.child.sendline('/draft');t.expect('Saved draft restored');t.expect('Do not dispatch automatically.')
    t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.command('/draft clear','Saved workspace draft removed')

    t.child.send('Explain this repository.');t.child.send('\x0a');t.child.sendline('Do not modify files.')
    t.expect('◇ Answered',timeout=35);t.expect('pactrail ❯')
    records=list((t.workspace/'.pactrail/runs').glob('*/run.json'))
    run=json.loads(records[0].read_text())
    traces=[json.loads(l) for l in records[0].with_name('trace.jsonl').read_text().splitlines()]
    contract=next(e['event']['data'] for e in traces if e['event']['type']=='contract_registered')
    assert '\nDo not modify files.' in contract['goal'],contract['goal']
    first_id=records[0].parent.name
    t.capture.snapshot('answer')
    assert any('continue: '+first_id[:13] in row for row in t.capture.screen.display), 'composer hid explicit continuation focus'
    t.child.sendline('/retry');t.expect('Previous task restored');t.expect('Do not modify files.');t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.command('/model fixture-edit','Model selected')
    t.command('Create candidate.md with a short test heading.','◇ Awaiting review')
    assert not (t.workspace/'candidate.md').exists()
    before_review=len(list((t.workspace/'.pactrail/runs').glob('*/run.json')))
    t.command('continue','Candidate awaiting review')
    assert len(list((t.workspace/'.pactrail/runs').glob('*/run.json')))==before_review
    assert not (t.workspace/'candidate.md').exists()
    t.capture.snapshot('review')
    t.command('/evidence','required')
    t.child.sendline('/apply');t.expect('Confirm');t.child.sendline('');t.expect('Apply cancelled');t.expect('pactrail ❯');assert not (t.workspace/'candidate.md').exists()
    t.child.sendline('/apply');t.expect('Confirm');t.child.sendline('yes');t.expect('Apply cancelled');t.expect('pactrail ❯');assert not (t.workspace/'candidate.md').exists()
    t.child.sendline('/apply');t.expect('Confirm');t.child.sendline('apply');t.expect('Applied 1 file');t.expect('pactrail ❯');assert (t.workspace/'candidate.md').read_text()=='# Test candidate\n'
    t.capture.snapshot('applied')
    t.command('/model fixture-read','Model selected')
    t.command('continue','◇ Answered')
    assert (t.workspace/'candidate.md').read_text()=='# Test candidate\n'
    t.command('/model fixture-edit','Model selected')
    (t.workspace/'candidate.md').unlink()
    t.command('Create candidate.md with a short test heading.','◇ Awaiting review')
    t.child.sendline('/discard');t.expect('Confirm');t.child.sendline('');t.expect('Discard cancelled');t.expect('pactrail ❯')
    t.child.sendline('/discard');t.expect('Confirm');t.child.sendline('discard');t.expect('Candidate discarded');t.expect('pactrail ❯');assert not (t.workspace/'candidate.md').exists()
    t.command('/focus '+first_id[:16],'Focused run')
    t.command('/inspect','◇ Answered')
    t.child.sendline('/trace');t.expect('Execution trace');time.sleep(.2);t.child.send('q');t.expect('pactrail ❯')
    t.child.send('\x10');t.expect('/resume');t.child.send('\x1b');time.sleep(.1);t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.child.send('/ev');t.child.send('\t');t.expect('/evidence');t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    task=t.workspace/'task with spaces.md';task.write_text('A loaded draft.\nSecond line.')
    t.child.send('/task task');t.child.send('\t');t.expect('task with spaces.md')
    t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.child.send('/evd');t.child.send('\t');t.expect('/evidence')
    t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.child.send('A draft before browsing runs.');t.child.send('\x0f');t.expect('Draft saved before switching runs');t.expect('Type part of a goal')
    t.child.send('Explain');t.child.send('\t');t.expect(first_id[:8])
    t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')

    t.child.sendline('/task "task with spaces.md"');t.expect('Task loaded');t.expect('Second line.');t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.child.send('An unfinished draft.');t.child.send('\x07');t.expect('Edited draft in external editor.');t.capture.snapshot('editor');t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.close()
    t.child = pexpect.spawn(str(BINARY), ['--workspace',str(t.workspace)],env=t.env,encoding='utf-8',dimensions=(34,100),timeout=20)
    t.child.linesep = "\r";t.child.logfile_read=t.capture
    t.expect('Saved workspace draft available');t.expect('pactrail ❯')
    t.child.sendline('/draft');t.expect('Saved draft restored');t.expect('A draft before browsing runs.')
    t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.child.sendline('/retry');t.expect('Previous task restored');t.expect('Create candidate.md')
    t.child.send('\x03');t.expect('Input cancelled');t.expect('pactrail ❯')
    t.command('/model fixture-read','Model selected')
    t.command('/focus '+first_id[:16],'Focused run')
    before=set(p.parent.name for p in (t.workspace/'.pactrail/runs').glob('*/run.json'))
    t.command('continue','◇ Answered')
    after=set(p.parent.name for p in (t.workspace/'.pactrail/runs').glob('*/run.json'))
    new_id=(after-before).pop()
    child_events=[json.loads(line) for line in (t.workspace/'.pactrail/runs'/new_id/'trace.jsonl').read_text().splitlines()]
    child_contract=next(event['event']['data'] for event in child_events if event['event']['type']=='contract_registered')
    assert 'Original user goal:' in child_contract['goal'] and 'Explain this repository.' in child_contract['goal']
    assert child_contract['allowed_write_paths']==contract['allowed_write_paths']
    assert child_contract['permissions']==contract['permissions']
    assert child_contract['budget']==contract['budget']
    for field in ['out_of_scope','obligations','acceptance_checks']:
        assert child_contract.get(field)==contract.get(field)
    assert '# Workspace overview' not in child_contract['goal']
    context_actions=[e['event']['data'] for e in child_events if e['event']['type']=='action_completed' and e['event']['data']['actor']=='context']
    assert any(int(action.get('attributes',{}).get('memory_fragments','0'))>=1 for action in context_actions)
    # A provider error leaves a durable checkpoint; plain continue recovers its ID.
    # The shared restart default must never remotely change live-session focus.
    t.command('/focus '+first_id[:16],'Focused run')
    discarded_id=next(json.loads(p.read_text())['run_id'] for p in (t.workspace/'.pactrail/runs').glob('*/receipt.json') if json.loads(p.read_text())['outcome']=='discarded')
    other=t.fork_session('second-session');terminals.append(other)
    other.command('/focus '+discarded_id[:16],'Focused run')
    other.close()
    before_cross_session=set((t.workspace/'.pactrail/runs').glob('*/run.json'))
    t.command('continue','◇ Answered')
    assert len(set((t.workspace/'.pactrail/runs').glob('*/run.json'))-before_cross_session)==1
    t.command('/model fixture-recover','Model selected')
    t.command('Explain after a transient provider failure.','Deliberate test provider failure')
    failed=[p for p in (t.workspace/'.pactrail/runs').glob('*/run.json') if not p.with_name('receipt.json').exists()]
    assert len(failed)==1
    failed_id=failed[0].parent.name
    before_recovery=len(list((t.workspace/'.pactrail/runs').glob('*/run.json')))
    t.close()
    t.child=pexpect.spawn(str(BINARY),['--workspace',str(t.workspace)],env=t.env,encoding='utf-8',dimensions=(34,100),timeout=20)
    t.child.linesep="\r";t.child.logfile_read=t.capture;t.expect('pactrail ❯')
    t.command('continue','◇ Answered')
    assert len(list((t.workspace/'.pactrail/runs').glob('*/run.json')))==before_recovery
    assert (t.workspace/'.pactrail/runs'/failed_id/'receipt.json').is_file()
    t.command('/continue forget','local answer memory')
    t.command('continue','Several tasks exist')
    t.command('/continue '+failed_id[:16],'◇ Answered')
    t.close()
    live=Terminal(width=100,name='live-input');terminals.append(live)
    live.command('/model fixture-slow','Model selected')
    live.child.sendline('Explain slowly while I draft the next task.');live.expect('Running')
    draft='Next 日本語 task.\nKeep this draft.'
    live.child.send('\x1b[200~'+draft+'\x1b[201~');live.expect('Keep this draft.')
    live.child.send('\x1b[D')
    live.expect('◇ Answered',timeout=35);live.expect('pactrail ❯')
    assert len(list((live.workspace/'.pactrail/runs').glob('*/run.json')))==1, 'type-ahead dispatched itself'
    live.child.send('X');live.child.send('\x13');live.expect('Workspace draft saved');live.expect('pactrail ❯')
    saved_draft=next((live.root/'config/composer').glob('*-draft.txt')).read_text()
    assert saved_draft=='Next 日本語 task.\nKeep this draftX.', 'streaming or completion moved the caret or lost pasted text: '+repr(saved_draft)
    live.child.send('\x03');live.expect('Input cancelled');live.expect('pactrail ❯')
    live.child.sendline('Explain another slow task.');live.expect('Running')
    live.child.sendline('This draft needs explicit dispatch.');live.expect('Draft retained')
    live.expect('◇ Answered',timeout=35);live.expect('pactrail ❯')
    assert len(list((live.workspace/'.pactrail/runs').glob('*/run.json')))==2
    live.child.send('\x03');live.expect('Input cancelled');live.expect('pactrail ❯')
    live.command('/model fixture-stop','Model selected')
    live.child.sendline('Wait so I can stop this task.');live.expect('Running')
    live.child.send('\x1b[200~Preserve my draft after stopping.\x1b[201~');live.expect('Preserve my draft')
    live.child.send('\x03');live.expect('Stop requested');live.expect('Stopped',timeout=35);live.expect('pactrail ❯')
    assert next((live.root/'config/composer').glob('*-draft.txt')).read_text()=='Preserve my draft after stopping.'
    stopped=[json.loads(p.read_text()) for p in (live.workspace/'.pactrail/runs').glob('*/receipt.json') if json.loads(p.read_text())['outcome']=='cancelled']
    assert len(stopped)==1
    live.child.send('\x03');live.expect('Input cancelled');live.expect('pactrail ❯')
    live.command('/model fixture-process','Model selected')
    live.command('/process native','Trusted native process execution enabled')
    live.child.sendline('Run the fixture command only with explicit approval.');live.expect('Running')
    live.child.send('run')
    live.expect(r'Type "once ([a-f0-9]{6})"');nonce=live.child.match.group(1)
    live.expect(r'Approval \(default: deny\)')
    live.child.sendline('')
    live.expect('Receipt · × Failed',timeout=35);live.expect('pactrail ❯')
    denied=sorted((live.workspace/'.pactrail/runs').glob('*/receipt.json'),key=lambda p:p.stat().st_mtime)[-1]
    denied_receipt=json.loads(denied.read_text())
    assert not denied_receipt['changes'], 'buffered text granted approval'
    assert any(record['decision']=='deny' for record in denied_receipt['approvals']), 'denial was not durably recorded'
    assert not (live.workspace/'permission-proof.txt').exists()
    live.child.send('\x03');live.expect('Input cancelled');live.expect('pactrail ❯')
    live.child.sendline('Run the fixture command with fresh consent.');live.expect(r'Type "once ([a-f0-9]{6})"');nonce=live.child.match.group(1)
    live.expect(r'Approval \(default: deny\)');live.child.sendline('once '+nonce)
    live.expect('◇ Awaiting review',timeout=35);live.expect('pactrail ❯')
    assert not (live.workspace/'permission-proof.txt').exists(), 'approved command escaped isolation'
    latest=sorted((live.workspace/'.pactrail/runs').glob('*/receipt.json'),key=lambda p:p.stat().st_mtime)[-1]
    assert any(change['path']=='permission-proof.txt' for change in json.loads(latest.read_text())['changes'])
    live.close()
    for width in [32,40,60,80,100,120,160]:
        narrow=Terminal(width=width,name='width-'+str(width));terminals.append(narrow)
        narrow.command('/help editor','/editor')
        narrow.close()
    dumb=Terminal(width=80,name='dumb',dumb=True);terminals.append(dumb);dumb.command('/clear','Plain mode preserves');dumb.close()
    assert '\x1b' not in dumb.capture.text, 'dumb mode emitted terminal control sequences'
    plain=Terminal(width=80,name='plain',plain=True);terminals.append(plain)
    plain.command('Explain this repository', '◇ Answered')
    plain.command('/retry','Previous task restored')
    before_plain=len(list((plain.workspace/'.pactrail/runs').glob('*/run.json')))
    plain.command('/dispatch', '◇ Answered')
    assert len(list((plain.workspace/'.pactrail/runs').glob('*/run.json')))==before_plain+1
    plain.command('/model fixture-edit','Model selected')
    plain.command('Create candidate.md','◇ Awaiting review')
    plain.child.sendline('/apply');plain.expect(r'Confirm \(default: cancel\) >')
    plain.child.sendline('');plain.expect('Apply cancelled');plain.expect('pactrail ❯')
    assert not (plain.workspace/'candidate.md').exists()
    plain.child.sendline('/apply');plain.expect(r'Confirm \(default: cancel\) >')
    plain.child.sendline('apply');plain.expect('Applied');plain.expect('pactrail ❯')
    assert (plain.workspace/'candidate.md').exists()
    plain.close();assert '\x1b' not in plain.capture.text, 'plain mode emitted terminal control sequences'
    ascii_terminal=Terminal(width=40,name='ascii',ascii=True);terminals.append(ascii_terminal)
    ascii_terminal.command('Explain this repository', '◇ Answered')
    ascii_terminal.capture.snapshot('ascii-answer')
    assert 'Composer .' in ascii_terminal.capture.text and '<> Answered' in ascii_terminal.capture.text
    ascii_terminal.close()
    for width in [40,100]:
        colored=Terminal(width=width,name='color-'+str(width),color=True);terminals.append(colored)
        colored.command('Explain this repository', '◇ Answered')
        colored.capture.snapshot('color-'+str(width)+'-answer')
        colored.command('/model fixture-edit','Model selected')
        colored.command('Create candidate.md with a short test heading.','◇ Awaiting review')
        colored.capture.snapshot('color-'+str(width)+'-review')
        colored.close()
        assert re.search(r'\x1b\[(?:3[0-7]|9[0-7])m',colored.capture.text), 'color terminal missing palette'
    (OUT/'results.json').write_text(json.dumps({'passed':['continue without history','pending candidate continue does not dispatch or apply','completed follow-up preserves contract','applied follow-up can answer without redundant edits','plain continue after provider failure and restart keeps run ID','forget local context and refuse ambiguous task selection','bracketed paste without dispatch','workspace draft save/restore/remove','draft and retry after restart','fuzzy command completion','run search by goal','task path completion','multiline dispatch','answer','draft retry','evidence','apply default cancel','apply incorrect acknowledgment cancel','explicit apply','discard default cancel','explicit discard','run focus','less pager return','command palette','Tab completion','task file draft','external editor draft','32/40/60/80/100/120/160 column startup','type-ahead preserves paste and caret without dispatch','Enter during execution saves instead of dispatching','Ctrl-C stops and preserves the draft','buffered draft cannot grant process approval','fresh process consent keeps changes isolated','cursor-free TERM=dumb','plain draft dispatch and guarded apply','ASCII composer and outcome','NO_COLOR','colored startup, answer and review at 40/100 columns','simultaneous sessions cannot redirect continue'],'artifacts':str(OUT)},indent=2))
    print((OUT/'results.json').read_text())
finally:
    for t in terminals:
        if t.child.isalive(): t.child.terminate(force=True)
        t.capture.log.close()
    provider.terminate();provider.wait(timeout=5)
