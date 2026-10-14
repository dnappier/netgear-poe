import importlib.util
import os
from pathlib import Path
import tempfile
import tomllib
import unittest

spec = importlib.util.spec_from_file_location('setup_config', Path(__file__).parents[1] / 'scripts/setup-config.py')
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class SetupTests(unittest.TestCase):
    def test_private_file_round_trip_and_no_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / '.credentials.toml'
            password = 'synthetic " quote \\ newline\n'
            setup.create_config(path, 'http://192.0.2.1', 'admin', password)
            self.assertEqual(tomllib.loads(path.read_text())['password'], password)
            self.assertEqual(os.stat(path).st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                setup.create_config(path, 'http://192.0.2.1', 'admin', 'replacement')
            self.assertEqual(tomllib.loads(path.read_text())['password'], password)

    def test_bad_origins_rejected(self):
        for url in ['ftp://192.0.2.1', 'http://admin:password@192.0.2.1', 'http://192.0.2.1/api', 'http://192.0.2.1?secret=x']:
            with self.assertRaises(ValueError):
                setup.create_config('/unused', url, 'admin', 'synthetic')
