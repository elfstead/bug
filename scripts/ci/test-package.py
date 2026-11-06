#!/usr/bin/env python3
"""Exercise the extracted release with no display or access to the engine build tree."""
import os
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile

root = Path(__file__).resolve().parents[2]
archive = Path(sys.argv[1]).resolve()
engine = root / 'build/engine'
hidden = root / 'build/engine.package-test-hidden'
if hidden.exists():
    raise RuntimeError(f'Restore {hidden} before retrying')
with tempfile.TemporaryDirectory(prefix='bug release test ') as temporary:
    work = Path(temporary)
    with tarfile.open(archive) as tar:
        tar.extractall(work, filter='data')
    binary = next(work.glob('bug-*/bin/bug'))
    for name in ('tests', 'examples', 'build'):
        (work / name).mkdir()
    for name in ('tests/headless.py', 'tests/features.py', 'tests/smoke.py',
                 'tests/capture_x11.py', 'examples/smoke.html'):
        shutil.copy2(root / name, work / name)
    env = os.environ.copy()
    for name in ('DISPLAY', 'WAYLAND_DISPLAY', 'LD_LIBRARY_PATH', 'DYLD_LIBRARY_PATH',
                 'DYLD_FALLBACK_LIBRARY_PATH', 'FONTCONFIG_FILE', 'FONTCONFIG_PATH'):
        env.pop(name, None)
    env['BUG_TEST_BINARY'] = str(binary)
    engine.rename(hidden)
    try:
        info = json.loads((binary.parent.parent / 'build.json').read_text())
        version = subprocess.check_output([binary, '--version'], cwd=work, env=env, text=True).strip()
        assert version == f'bug {info["build_id"]} ({info["commit"]})', version
        for test in ('headless.py', 'features.py'):
            subprocess.run([sys.executable, work / 'tests' / test], cwd=work, env=env,
                           timeout=240, check=True)
        if platform.system() == 'Linux':
            subprocess.run(['xvfb-run', '-a', '-s', '-screen 0 1280x1024x24', sys.executable,
                            work / 'tests/smoke.py'], cwd=work, env=env, timeout=240, check=True)
    finally:
        hidden.rename(engine)
print('PASS: relocated archive renders, evaluates JS, handles clipboard primitives and restores sessions')
