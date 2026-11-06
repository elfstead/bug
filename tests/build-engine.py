#!/usr/bin/env python3
"""Exercise the build wrapper with platform Bash, without compiling an engine."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='bug build arguments ') as temporary:
    work = Path(temporary)
    scripts = work / 'scripts'
    scripts.mkdir()
    shutil.copy2(root / 'scripts/build-engine', scripts / 'build-engine')
    prepare = scripts / 'prepare-engine'
    prepare.write_text('#!/bin/sh\nexit 0\n')
    prepare.chmod(0o755)
    cmake = work / 'cmake'
    cmake.write_text(f'''#!{sys.executable}
import json, os, sys
with open(os.environ['BUG_TEST_LOG'], 'a') as stream:
    stream.write(json.dumps(sys.argv[1:]) + '\\n')
''')
    cmake.chmod(0o755)
    log = work / 'calls.jsonl'
    env = os.environ.copy()
    env.pop('BUG_ICU_ROOT', None)
    env['PATH'] = str(work) + os.pathsep + env['PATH']
    env['BUG_TEST_LOG'] = str(log)
    env['BUG_JOBS'] = '3'
    for dependencies, icu, arguments in [
        ('vcpkg', '', []),  # Previously fails with Bash 3.2: empty array + nounset.
        ('system', '', ['-DCMAKE_PREFIX_PATH=/explicit path']),
        ('vcpkg', '/icu path', ['-DCMAKE_OSX_DEPLOYMENT_TARGET=15.0']),
    ]:
        env['BUG_DEPENDENCIES'] = dependencies
        env['BUG_ICU_ROOT'] = icu
        log.write_text('')
        subprocess.run([scripts / 'build-engine', *arguments], env=env, check=True)
        configure, build = [json.loads(line) for line in log.read_text().splitlines()]
        assert configure[:4] == ['-S', str(work / 'vendor/ladybird'), '-B', str(work / 'build/engine')]
        assert '' not in configure
        if arguments:
            assert configure[-len(arguments):] == arguments  # Caller overrides remain last.
        if dependencies == 'system':
            assert f'-DCMAKE_PREFIX_PATH={work}/build/deps' in configure
        else:
            assert not any(arg.startswith('-DCMAKE_PREFIX_PATH=') for arg in configure)
        if icu:
            assert f'-DICU_ROOT={icu}' in configure
        assert build == ['--build', str(work / 'build/engine'), '--target', 'bug_engine', '--parallel', '3']
print('PASS: vcpkg with no optional arguments, system dependencies, ICU override, quoting and caller precedence')
