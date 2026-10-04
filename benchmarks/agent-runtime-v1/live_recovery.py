#!/usr/bin/env python3
"""Unscored hosted-model crash/resume and cancellation qualification."""
import argparse
import json
import os
import re
from pathlib import Path
import signal
import sqlite3
import subprocess
import time
from live_qualification import configuration


def events(root):
    database = root / 'state/events.sqlite3'
    if not database.exists():
        return []
    try:
        with sqlite3.connect(f'file:{database}?mode=ro', uri=True) as connection:
            return [{'run_id': run, 'sequence': sequence, 'event': json.loads(event)}
                    for run, sequence, event in connection.execute('SELECT run_id, sequence, event_json FROM events ORDER BY sequence')]
    except sqlite3.OperationalError:
        return []  # initial schema transaction may not yet have committed


def wait_invocation(child, root):
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline and child.poll() is None:
        rows = events(root)
        if rows and any(r['event']['type'] == 'action_completed' and r['event']['data']['action'] == 'agent_started' for r in rows) and rows[-1]['event']['type'] == 'checkpoint_created':
            return rows[0]['run_id']
        time.sleep(.02)
    raise RuntimeError('no reserved provider boundary observed before process exit/deadline')


def run(output, env_file):
    identity, env = configuration(env_file)
    binary = Path(env['PACTRAIL_BENCH_BINARY']).resolve(strict=True)
    output.mkdir(parents=True, exist_ok=False)
    results = []
    for mode in ('crash-resume', 'cancel'):
        root = output / mode
        workspace = root / 'workspace'
        workspace.mkdir(parents=True)
        (workspace / 'value.txt').write_text('before\n')
        base = [str(binary), '--workspace', str(workspace), '--state-dir', str(root / 'state')]
        profile = root / 'agents.json'
        profile.write_bytes(subprocess.check_output(base + ['agent-template']))
        task = root / 'task.toml'
        template = subprocess.check_output(base + ['task-template', 'Read value.txt, change it to after, and give a concise result. Commands are disabled.'], text=True)
        template = re.sub(r'(?m)^wall_time_seconds\s*=\s*\d+\s*$', 'wall_time_seconds = 600', template)
        task.write_text(template)
        argv = base + ['run', '--task', str(task), '--agent-config', str(profile), '--output', 'json', '--no-stream', '--max-turns', '24', '--context-tokens', '65536', '--max-output-tokens', '8192', '--reasoning-effort', 'low']
        for key, value in identity.items():
            argv += ['--' + key.replace('_', '-'), value]
        with (root / 'stdout.log').open('wb') as stdout, (root / 'stderr.log').open('wb') as stderr:
            child = subprocess.Popen(argv, env=env, stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                run_id = wait_invocation(child, root)
                os.killpg(child.pid, signal.SIGKILL if mode == 'crash-resume' else signal.SIGINT)
                child.wait(timeout=30)
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait()
        if mode == 'crash-resume':
            with (root / 'resume.stdout').open('wb') as stdout, (root / 'resume.stderr').open('wb') as stderr:
                resumed = subprocess.run(base + ['resume', run_id, '--output', 'json'], env=env, stdout=stdout, stderr=stderr, timeout=650, check=False)
            exit_code = resumed.returncode
        else:
            exit_code = child.returncode
        rows = events(root)
        receipts = list((root / 'state').rglob('receipt.json'))
        receipt = json.loads(receipts[0].read_text()) if len(receipts) == 1 else None
        actions = [r['event']['data'] for r in rows if r['event']['type'] == 'action_completed']
        source_unchanged = (workspace / 'value.txt').read_text() == 'before\n'
        outcome = receipt['outcome'] if receipt else None
        expected = 'ready_to_apply' if mode == 'crash-resume' else 'cancelled'
        agents = subprocess.run(base + ['agents', run_id, '--json'], env=env, capture_output=True, check=False)
        (root / 'agents.json.out').write_bytes(agents.stdout)
        (root / 'agents.stderr').write_bytes(agents.stderr)
        row = {'mode': mode, 'model': identity, 'run_id': run_id, 'exit_code': exit_code,
               'outcome': outcome, 'source_unchanged': source_unchanged,
               'successful_writes': sum(a['actor'] == 'tool:write_file' and a['succeeded'] for a in actions),
               'agents_valid': agents.returncode == 0,
               'verdict': 'PASS' if source_unchanged and outcome == expected and agents.returncode == 0 else 'NOT QUALIFIED'}
        results.append(row)
        (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
        print(mode, row['verdict'], flush=True)
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--env-file', type=Path)
    args = parser.parse_args()
    run(args.output.resolve(), args.env_file)
