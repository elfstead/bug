#!/usr/bin/env python3
"""Bundle the native host, helpers and their dependency closure; never require a checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
from version import build_info

ROOT = Path(__file__).resolve().parents[2]
ENGINE = ROOT / 'build/engine'
HELPERS = ('AudioServer', 'Compositor', 'ImageDecoder', 'MediaServer', 'RequestServer',
           'VideoDecoderServer', 'WebContent', 'WebRTCClient', 'WebWorker')
GLIBC = re.compile(r'^(ld-linux.*|lib(c|m|pthread|dl|rt|resolv|util|anl)\.so\..*)$')


def run(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True).strip()


def copy(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        if source.read_bytes() != destination.read_bytes():
            raise RuntimeError(f'Conflicting library basenames: {source} and {destination}')
    else:
        shutil.copy2(source, destination, follow_symlinks=True)
    return destination


def linux_dependencies(binary, search=()):
    env = os.environ.copy()
    # Use one consistent dependency set when inspecting distro libraries that
    # also depend on vcpkg-provided SONAMEs (zstd, zlib, etc.). The final audit
    # deliberately has no search override and tests the rewritten RPATHs.
    env.pop('LD_LIBRARY_PATH', None)
    if search:
        env['LD_LIBRARY_PATH'] = os.pathsep.join(str(path) for path in search)
    output = subprocess.check_output(['ldd', str(binary)], text=True, env=env)
    if 'not found' in output:
        raise RuntimeError(f'Unresolved dependencies for {binary}:\n{output}')
    return [Path(match) for match in re.findall(r'(?:=>\s+|^\s*)(/\S+)\s+\(', output, re.M)]


def mac_dependencies(binary):
    return [line.strip().split(' (compatibility version', 1)[0]
            for line in run('otool', '-L', binary).splitlines()[1:]]


def mac_rpaths(binary):
    return re.findall(r'cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset', run('otool', '-l', binary))


def mac_system(path):
    return path.startswith(('/System/Library/', '/usr/lib/'))


def mac_resolve(name, binary, search):
    def expand(value):
        return value.replace('@loader_path', str(binary.parent)).replace('@executable_path', str(ENGINE / 'bin'))
    if name.startswith('@rpath/'):
        suffix = name.removeprefix('@rpath/')
        candidates = [Path(expand(p)) / suffix for p in mac_rpaths(binary)]
        candidates += [directory / suffix for directory in search]
    else:
        candidates = [Path(expand(name))]
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise RuntimeError(f'Cannot resolve {name} from {binary}')


def licenses(destination, sources, triplet):
    base = destination / 'licenses'
    copy(ROOT / 'LICENSE', base / 'bug/LICENSE')
    copy(ROOT / 'vendor/ladybird/LICENSE', base / 'ladybird/LICENSE')
    # Ladybird's bundled third-party source licenses are separate from vcpkg.
    for directory in (ROOT / 'vendor/ladybird/Libraries', ROOT / 'vendor/ladybird/AK',
                      ROOT / 'vendor/ladybird/Services'):
        for source in directory.rglob('*'):
            if source.is_file() and source.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE')):
                copy(source, base / 'ladybird' / source.relative_to(ROOT / 'vendor/ladybird'))
    installed = ENGINE / 'vcpkg_installed' / triplet / 'share'
    if not installed.exists():
        raise RuntimeError('Release packaging requires vcpkg dependencies')
    for source in installed.glob('*/copyright'):
        copy(source, base / 'vcpkg' / source.parent.name / 'copyright')
    rust = ROOT / 'build/rust-vendor'
    if not rust.exists():
        raise RuntimeError('Run cargo vendor before packaging dependency licenses')
    for crate in rust.iterdir():
        if not crate.is_dir():
            continue
        copy(crate / 'Cargo.toml', base / 'rust' / crate.name / 'Cargo.toml')
        for source in crate.rglob('*'):
            if source.is_file() and source.name.upper().startswith(('LICENSE', 'COPYING', 'NOTICE', 'COPYRIGHT')):
                copy(source, base / 'rust' / crate.name / source.relative_to(crate))
    if platform.system() == 'Linux':
        for source in sources:
            if not str(source).startswith(('/usr/', '/lib/')):
                continue
            # Resolve usrmerge aliases when asking dpkg which package owns a library.
            owner = None
            for candidate in dict.fromkeys((str(source), str(source.resolve()), str(source).replace('/usr/lib/', '/lib/'))):
                result = subprocess.run(['dpkg-query', '-S', candidate], text=True, capture_output=True)
                if result.returncode == 0:
                    owner = result.stdout.split(': ', 1)[0].split(':', 1)[0]
                    break
            if owner is None:
                raise RuntimeError(f'Cannot find distro license for {source}')
            copy(Path('/usr/share/doc') / owner / 'copyright', base / 'system' / owner / 'copyright')


def package(target):
    info = build_info()
    mac = target == 'macos-arm64'
    assert (platform.system(), platform.machine()) == (('Darwin', 'arm64') if mac else ('Linux', 'x86_64'))
    triplet = 'arm64-osx-dynamic' if mac else 'x64-linux-dynamic'
    destination = ROOT / 'dist' / f'bug-{info["build_id"]}-{target}'
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    libraries = destination / 'lib'
    libraries.mkdir()
    search = [ENGINE / 'lib', ENGINE / 'vcpkg_installed' / triplet / 'lib']
    sources = {}  # destination -> original, so resolution uses original loader paths
    def add(source, output):
        if 'qt' in source.name.lower():
            raise RuntimeError(f'Unexpected Qt dependency: {source}')
        copy(source, output)
        sources.setdefault(output, source)
    add(ROOT / 'target/release/bug', destination / 'bin/bug-host')
    for helper in (*HELPERS, *(('ProcessReaper',) if mac else ())):
        add(ENGINE / 'bin' / helper, destination / 'bin' / helper)
    add(ENGINE / 'lib' / ('libbug_engine.dylib' if mac else 'libbug_engine.so'),
        libraries / ('libbug_engine.dylib' if mac else 'libbug_engine.so'))
    # winit loads these at runtime, so they don't appear in the ELF dependency graph.
    if not mac:
        cache = run('ldconfig', '-p')
        for name in ('libwayland-client.so.0', 'libxkbcommon.so.0', 'libxkbcommon-x11.so.0',
                     'libX11.so.6', 'libX11-xcb.so.1',
                     'libXcursor.so.1', 'libXi.so.6', 'libXrandr.so.2', 'libxcb.so.1'):
            preferred = next((directory / name for directory in search if (directory / name).is_file()), None)
            match = re.search(r'^\s*' + re.escape(name) + r'\s+.*?=> (\S+)', cache, re.M)
            if preferred is None and not match:
                raise RuntimeError(f'Missing runtime windowing dependency {name}')
            add(preferred if preferred is not None else Path(match[1]), libraries / name)
    processed = set()
    links = {}
    while pending := [path for path in sources if path not in processed]:
        for output in pending:
            source = sources[output]
            processed.add(output)
            if mac:
                links[output] = {}
                for name in mac_dependencies(source):
                    if mac_system(name):
                        continue
                    dependency = mac_resolve(name, source, search)
                    bundled = libraries / dependency.name
                    add(dependency, bundled)
                    links[output][name] = '@loader_path/' + os.path.relpath(bundled, output.parent)
            else:
                for dependency in linux_dependencies(source, search):
                    if not GLIBC.match(dependency.name):
                        add(dependency, libraries / dependency.name)
    licenses(destination, sources.values(), triplet)
    for output in sources:
        output.chmod(output.stat().st_mode | 0o200)
        if mac:
            for old, new in links[output].items():
                run('install_name_tool', '-change', old, new, output)
            for rpath in set(mac_rpaths(output)):
                run('install_name_tool', '-delete_rpath', rpath, output)
            if output.parent == libraries:
                run('install_name_tool', '-id', '@rpath/' + output.name, output)
        else:
            if '/nix/store/' in run('patchelf', '--print-rpath', output):
                raise RuntimeError('Nix binaries are not portable release inputs; build with scripts/ci/build-release')
            run('patchelf', '--set-rpath', '$ORIGIN' if output.parent == libraries else '$ORIGIN/../lib', output)
    resource_path = Path('Resources' if mac else 'share/Lagom')
    shutil.copytree(ENGINE / resource_path, destination / resource_path, symlinks=False)
    copy(ROOT / 'README.md', destination / 'README.md')
    copy(ROOT / 'LICENSE', destination / 'LICENSE')
    # Fontconfig's vcpkg install prefix won't exist on the recipient's machine.
    (destination / 'fonts.conf').write_text('''<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig><dir>/usr/share/fonts</dir><dir>/usr/local/share/fonts</dir>
<dir>/System/Library/Fonts</dir><dir>/Library/Fonts</dir><dir>~/Library/Fonts</dir>
<dir prefix="xdg">fonts</dir><cachedir prefix="xdg">fontconfig</cachedir></fontconfig>
''')
    launcher = destination / 'bin/bug'
    launcher.write_text('''#!/bin/sh
bug_prefix=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -z "${FONTCONFIG_FILE:-}" ]; then
    if [ -f /etc/fonts/fonts.conf ]; then
        FONTCONFIG_FILE=/etc/fonts/fonts.conf
    else
        FONTCONFIG_FILE="$bug_prefix/fonts.conf"
    fi
    export FONTCONFIG_FILE
fi
exec "$bug_prefix/bin/bug-host" "$@"
''')
    launcher.chmod(0o755)
    if mac:
        for output in sources:
            args = ['codesign', '--force', '--sign', '-']
            if output.parent.name == 'bin':
                # Preserve upstream's per-helper JIT permissions after relocation.
                entitlements = (ROOT / 'scripts/ci/macos.entitlements' if output.name == 'bug-host'
                                else ENGINE / 'Services' / output.name / f'{output.name}.entitlements')
                args += ['--options', 'runtime', '--entitlements', entitlements]
            run(*args, output)
            run('codesign', '--verify', '--strict', output)
    # Reject any remaining build-machine references in the runtime dependency graph.
    for output in sources:
        dependencies = mac_dependencies(output) if mac else [str(path) for path in linux_dependencies(output)]
        for dependency in dependencies:
            if mac:
                assert dependency.startswith('@loader_path/') or dependency.startswith('@rpath/') or mac_system(dependency), dependency
            elif not GLIBC.match(Path(dependency).name):
                assert Path(dependency).resolve().is_relative_to(destination), dependency
    metadata = {**info, 'platform': target,
                'ladybird_revision': (ROOT / 'ladybird-revision').read_text().strip()}
    (destination / 'build.json').write_text(json.dumps(metadata, indent=2) + '\n')
    archive = Path(f'{destination}.tar.gz')
    with tarfile.open(archive, 'w:gz') as tar:
        tar.add(destination, arcname=destination.name)
    print(f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=['linux-x64', 'macos-arm64'], required=True)
    package(parser.parse_args().platform)
