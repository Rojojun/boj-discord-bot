import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('audit', Path(__file__).resolve().parents[1] / 'scripts/audit-runtime.py')
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class AuditTest(unittest.TestCase):
    def test_three_accounts_and_no_secrets_in_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for user in range(1, 4):
                directory = root / 'users' / str(user)
                directory.mkdir(parents=True)
                (directory / 'settings.json').write_text(json.dumps({'delivery': 'dm', 'reminder_time': '21:00'}))
                (directory / 'solved-ac-session.json').write_text(json.dumps({'account': {'id': user}, 'expires_at': 0, 'tokens': {'scope': 'solves offline.access', 'access_token': 'DO-NOT-PRINT-ACCESS', 'refresh_token': 'DO-NOT-PRINT-REFRESH'}}))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertTrue(audit.audit(root))
            self.assertNotIn('DO-NOT-PRINT', output.getvalue())
            self.assertIn('서로 다른 연결 계정 3', output.getvalue())
            session = root / 'users/3/solved-ac-session.json'
            value = json.loads(session.read_text())
            value['account']['id'] = 1
            session.write_text(json.dumps(value))
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertFalse(audit.audit(root))

    def test_corrupt_session_reports_no_raw_content(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            directory = root / 'users/1'
            directory.mkdir(parents=True)
            (directory / 'settings.json').write_text('{}')
            (directory / 'solved-ac-session.json').write_text('SECRET-BROKEN-JSON')
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertFalse(audit.audit(root))
            self.assertNotIn('SECRET-BROKEN-JSON', output.getvalue())


if __name__ == '__main__':
    unittest.main()
