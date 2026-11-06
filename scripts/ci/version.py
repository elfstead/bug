#!/usr/bin/env python3
"""Use source commits for snapshots; explicit version tags designate milestones."""
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def build_info():
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if not re.fullmatch(r'[0-9a-f]{40,64}', commit):
        raise ValueError('Expected a full Git commit ID')
    tag = os.environ.get('GITHUB_REF_NAME', '') if os.environ.get('GITHUB_REF_TYPE') == 'tag' else ''
    if tag and not re.fullmatch(r'v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', tag):
        raise ValueError(f'Unsupported milestone tag: {tag!r}')
    build_id = tag or f'snapshot-{commit[:12]}'
    return {'build_id': build_id, 'commit': commit,
            'prerelease': not tag or '-' in tag, 'tag': tag or None}


if __name__ == '__main__':
    info = build_info()
    print(json.dumps(info, indent=2))
    if output := os.environ.get('GITHUB_OUTPUT'):
        with open(output, 'a') as stream:
            for key in ('build_id', 'commit', 'prerelease'):
                value = str(info[key]).lower() if isinstance(info[key], bool) else info[key]
                stream.write(f'{key}={value}\n')
