#!/usr/bin/env python3
"""Use the pinned engine's dependency versions, omitting its Qt frontend."""
import json
from pathlib import Path

root = Path(__file__).resolve().parents[2]
source = root / 'vendor/ladybird'
destination = root / 'build/release-manifest'
destination.mkdir(parents=True, exist_ok=True)
manifest = json.loads((source / 'vcpkg.json').read_text())
for field in ('dependencies', 'overrides'):
    manifest[field] = [entry for entry in manifest[field]
                       if not (entry if isinstance(entry, str) else entry['name']).startswith('qt')]
manifest['name'] = 'bug-engine'
manifest['version-string'] = (root / 'ladybird-revision').read_text().strip()
(destination / 'vcpkg.json').write_text(json.dumps(manifest, indent=2) + '\n')
configuration = json.loads((source / 'vcpkg-configuration.json').read_text())
configuration['overlay-ports'] = [str((source / path).resolve()) for path in configuration['overlay-ports']]
(destination / 'vcpkg-configuration.json').write_text(json.dumps(configuration, indent=2) + '\n')
print(f'Prepared Qt-free dependencies at {destination}')
