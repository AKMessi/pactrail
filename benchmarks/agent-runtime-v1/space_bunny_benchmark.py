#!/usr/bin/env python3
"""Freeze or execute the diagnostic historical Space Bunny comparison."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import urllib.request
from live_qualification import configuration
from run import run, validate


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def freeze(root, protocol, binary, env_file):
    if protocol.exists():
        raise ValueError('never overwrite a preregistration; use a new protocol identity')
    tools = Path(__file__).resolve().parent
    repository = tools.parent.parent
    manifest = root / 'issues.json'
    issues = json.loads(manifest.read_text())
    identity, _ = configuration(env_file)
    identity.update(reasoning_effort='low', require_response_model=True)
    # This is public provider metadata; credentials are never sent to this URL.
    with urllib.request.urlopen('https://openrouter.ai/api/v1/models', timeout=30) as response:
        metadata = next(row for row in json.load(response)['data'] if row['id'] == identity['model'])
    tasks = []
    for issue in issues:
        task = {key: issue[key] for key in ('id', 'repository', 'commit')}
        task['grader_timeout_seconds'] = max(issue['targeted_test']['timeout_seconds'], issue['regression_test']['timeout_seconds'])
        task['goal'] = issue['prompt'].replace('Run relevant tests before finishing.', '') + ' Commands are disabled; do not claim tests ran. An external grader will run the checks.'
        for phase in ('targeted', 'regression'):
            task[phase] = [sys.executable, str(tools / 'issue_grader.py'), str(manifest), issue['id'], phase]
        tasks.append(task)
    files = list(tools.glob('*.py')) + [manifest, binary]
    for issue in issues:
        files += [Path(issue['overlays']) / p for p in issue['overlay_sha256']]
        lock = Path(issue['baseline']) / 'Cargo.lock'
        if lock.exists():
            files.append(lock)
    protocol_data = {
        'schema_version': 1, 'seed': 210105, 'repetitions': 3,
        'runtime_identity': {'commit': subprocess.check_output(['git', '-C', str(repository), 'rev-parse', 'HEAD'], text=True).strip(), 'binary_sha256': sha(binary)},
        'model_identity': identity, 'provider_metadata': metadata,
        'limits': {'model_turns': 24, 'wall_seconds': 600, 'output_tokens': 8192, 'context_tokens': 65536, 'model_tokens': 500000},
        'permissions': {'process': 'disabled', 'write_paths': ['.']},
        'normalization': 'Equal global engine model attempts, cumulative token limit and wall budget. Text specialists share the global budget; read-only role ceilings and serialized implementer effects. Provider retries are not independently counted.',
        'capabilities': {'native_tools': True, 'parallel_tools': False, 'structured_output': False, 'vision': False, 'prompt_caching': False, 'streaming': False, 'reasoning_controls': True, 'source': 'user_declared'},
        'temperature': 0.0,
        'arms': [{'id': mode, 'mode': mode, 'profile': 'full' if mode == 'text' else None, 'adapter': [sys.executable, str(tools / 'pactrail_cli.py')]} for mode in ('single', 'text')],
        'tasks': tasks,
        'topology': 'Serial localizer -> solver -> critic -> implementer, one round; default template ceilings 4/4/4/12, global 24. Independent conversations; specialist tools read-only. Single uses the same tool kernel/contract with no peer messages.',
        'task_provenance': [{k: issue[k] for k in ('id', 'repository_url', 'issue_url', 'base_commit', 'reference_commit', 'commit', 'overlay_sha256')} for issue in issues],
        'population': 'Three public historical diagnostic defects, not independently curated held-out tasks. Synthetic baseline commits remove future history/remotes. Bytes stable autobenches=false; standalone bytes/fd workspace normalization. fd regression omits two previously documented platform-dependent tests. Gold reference tests are held outside agent workspaces.',
        'stopping_rules': 'All 18 declared trials count. Seed-shuffled sequential execution. No retries of failed trials, no substitution, no early performance stopping. Provider errors/timeouts are failures; invalid and unsupported remain reported separately. Any runner/grader correctness repair freezes this experiment and requires a new protocol.',
        'success_criteria': 'Both unchanged external targeted and regression graders pass on the isolated candidate; valid receipt/trace and source isolation are checked separately. No success claim from model prose.',
        'analysis': 'Paired task-level success delta with 10000 task-clustered bootstrap samples, seed 210105. Report all arm counts, per-task outcomes, covered token/cache/tool/communication totals and equally weighted task means; no synthetic score. Three tasks produce weak generalization/uncertainty evidence.',
        'limitations': ['Input/output budget checks occur after provider responses and may overshoot by one response.', 'HTTP adapter retries up to three times within one logical engine attempt; unavailable failed-request usage/billing cannot be reconstructed.', 'No exact inter-agent language-token count without generated_text_tokens; report null. Cache creation, GPU time, inference RSS and invoice cost are unknown.', 'No stronger process authority for either arm; both rely on external verification.', 'Opaque hosted checkpoint identity cannot be cryptographically attested; every reported response model must match the requested model.'],
        'environment': {'platform': platform.platform(), 'machine': platform.machine(), 'python': sys.version, 'cpu_count': os.cpu_count(), 'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(), 'hardware_cpu': next((l.split(':',1)[1].strip() for l in Path('/proc/cpuinfo').read_text().splitlines() if l.startswith('model name')), None)},
        'frozen_inputs': {str(p.resolve()): sha(p) for p in files},
    }
    validate(protocol_data)
    protocol.parent.mkdir(parents=True, exist_ok=True)
    protocol.write_text(json.dumps(protocol_data, indent=2, allow_nan=False) + '\n')
    protocol.with_suffix('.sha256').write_text(sha(protocol) + '  ' + protocol.name + '\n')
    print('Frozen:', protocol, sha(protocol))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--protocol', type=Path, required=True)
    parser.add_argument('--freeze', type=Path, help='prepared/validated historical task directory; freeze only, no model requests')
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--env-file', type=Path)
    args = parser.parse_args()
    if args.freeze:
        freeze(args.freeze.resolve(), args.protocol.resolve(), args.binary.resolve(strict=True), args.env_file)
    else:
        if args.output is None:
            parser.error('--output is required for execution')
        identity, env = configuration(args.env_file)
        frozen = json.loads(args.protocol.read_text())['model_identity']
        if any(frozen[k] != v for k, v in identity.items()):
            raise ValueError('existing configuration differs from frozen provider identity')
        os.environ.update({identity['api_key_env']: env[identity['api_key_env']], 'PACTRAIL_BENCH_BINARY': str(args.binary.resolve(strict=True))})
        run(args.protocol.resolve(), args.output.resolve())
