import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("agy_worker", ROOT / "scripts/agy-worker.py")
worker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(worker)


class WorkerTests(unittest.TestCase):
    def fake(self, home, body):
        binary = home / "fake-agy"
        binary.write_text(f"#!{sys.executable}\n" + body)
        binary.chmod(0o700)
        (home / "work").mkdir(exist_ok=True)
        return binary

    def test_request_rejects_private_fields_and_policy_matches_deployment(self):
        request = {"task": "goal", "goal": "LRU를 배우고 싶어요", "tracks": [{"id": "cache", "description": "캐시"}]}
        prompt, schema = worker.prepare(request)
        self.assertIn("LRU", prompt)
        self.assertEqual(schema, worker.GOAL_SCHEMA)
        for private in ("discord_id", "handle", "access_token", "history"):
            with self.assertRaises(ValueError):
                worker.prepare(dict(request, **{private: "private"}))
        with self.assertRaises(ValueError):
            worker.prepare(dict(request, goal="x" * 501))
        self.assertEqual(json.loads((ROOT / "deploy/agy-settings.json").read_text()), worker.POLICY)

    def test_subprocess_uses_argument_array_clean_environment_and_separate_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            binary = self.fake(home, 'import os,sys,json\nprint(json.dumps({"status":"SUCCESS","structured_output":{"secret_present":"DISCORD_BOT_TOKEN" in os.environ,"cwd":os.getcwd(),"args":sys.argv[1:]}}))\n')
            with patch.dict(os.environ, {"DISCORD_BOT_TOKEN": "fixture-secret", "SOLVED_AC_CLIENT_SECRET": "fixture-secret"}):
                result = worker.run_agy(binary, "test-model", home, "text; do not run shell", worker.GOAL_SCHEMA)
            self.assertFalse(result["secret_present"])
            self.assertTrue(Path(result["cwd"]).is_relative_to((home / "work").resolve()))
            self.assertIn("--sandbox", result["args"])
            self.assertIn("--json-schema", result["args"])
            self.assertNotIn("--dangerously-skip-permissions", result["args"])
            self.assertEqual(result["args"][1], "text; do not run shell")
            self.assertEqual(list((home / "work").iterdir()), [])

    def test_timeout_kills_process_instead_of_hanging(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            binary = self.fake(home, "import time\ntime.sleep(10)\n")
            start = time.monotonic()
            with self.assertRaisesRegex(ValueError, "timeout"):
                worker.run_agy(binary, "test-model", home, "test", worker.GOAL_SCHEMA, timeout=0.1)
            self.assertLess(time.monotonic() - start, 3)

    def test_output_limit_and_failed_envelope_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            binary = self.fake(home, "import sys\nsys.stdout.write('x'*65537)\n")
            with self.assertRaisesRegex(ValueError, "output_too_large"):
                worker.run_agy(binary, "test-model", home, "test", worker.GOAL_SCHEMA)
            binary = self.fake(home, 'print(\'{"status":"ERROR","structured_output":{}}\')\n')
            with self.assertRaisesRegex(ValueError, "invalid_response"):
                worker.run_agy(binary, "test-model", home, "test", worker.GOAL_SCHEMA)


if __name__ == "__main__":
    unittest.main()
