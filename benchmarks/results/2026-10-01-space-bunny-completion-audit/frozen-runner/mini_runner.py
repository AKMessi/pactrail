"""Unmodified mini agent and official LiteLLM adapter; endpoint is the shared proxy."""
import json, os, pathlib, sys
import yaml
from minisweagent.agents.default import DefaultAgent
from minisweagent.environments.local import LocalEnvironment
from minisweagent.models.litellm_model import LitellmModel
from minisweagent.config import builtin_config_dir
config=yaml.safe_load((builtin_config_dir/'mini.yaml').read_text())
agent_config=config['agent']
# Interactive-only options do not apply to the official noninteractive agent.
for key in ['mode','confirm_exit','whitelist_actions','blacklist_actions']:
    agent_config.pop(key,None)
agent_config.update(step_limit=12,cost_limit=0,output_path=sys.argv[2])
model_config=config.get('model',{})
model_config.update(model_name='openai/space-bunny-alpha',cost_tracking='ignore_errors',
    model_kwargs={'api_base':os.environ['BENCHMARK_PROXY_URL']+'/v1','api_key':os.environ['BENCHMARK_PROXY_KEY'],
                  'temperature':0,'max_tokens':8192})
model=LitellmModel(**model_config)
agent=DefaultAgent(model,LocalEnvironment(cwd=str(pathlib.Path.cwd()),timeout=30),**agent_config)
result=agent.run(pathlib.Path(sys.argv[1]).read_text())
print(json.dumps({'result':result},default=str))
