#!/usr/bin/env python3
"""Check that development builds cannot be mistaken for stable milestones."""
import importlib.util
import os
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('release_version', Path(__file__).resolve().parents[1] / 'scripts/ci/version.py')
version = importlib.util.module_from_spec(spec)
spec.loader.exec_module(version)
COMMIT = '0123456789abcdef0123456789abcdef01234567'


class ReleaseIdentity(unittest.TestCase):
    def info(self, ref_type='', ref_name=''):
        with patch.dict(os.environ, {'GITHUB_REF_TYPE': ref_type, 'GITHUB_REF_NAME': ref_name}, clear=True):
            with patch.object(version.subprocess, 'check_output', return_value=COMMIT + '\n'):
                return version.build_info()

    def test_local_and_branch_builds_are_commit_snapshots(self):
        for kind, name in [('', ''), ('branch', 'master'), ('branch', 'v0.1.0')]:
            self.assertEqual(self.info(kind, name), {
                'build_id': 'snapshot-0123456789ab', 'commit': COMMIT,
                'prerelease': True, 'tag': None,
            })

    def test_alpha_is_a_prerelease(self):
        info = self.info('tag', 'v0.1.0-alpha.1')
        self.assertEqual(info['build_id'], 'v0.1.0-alpha.1')
        self.assertTrue(info['prerelease'])

    def test_only_explicit_milestone_is_stable(self):
        info = self.info('tag', 'v0.1.0')
        self.assertEqual(info['tag'], 'v0.1.0')
        self.assertFalse(info['prerelease'])

    def test_invalid_tags_cannot_become_paths_or_releases(self):
        for tag in ['vnext', 'v0.1.0/../../other', 'v0.1.0\nbuild_id=oops']:
            with self.assertRaises(ValueError):
                self.info('tag', tag)


if __name__ == '__main__':
    unittest.main()
