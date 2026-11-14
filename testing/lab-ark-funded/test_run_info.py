"""Read-only local HTTP fixture; mocked process bindings are not live evidence."""
import contextlib
import io
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from common import Blocked
from run_info import publish_run_info


class RunInfo(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ap5-run-info-')
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name);(root / 'ap5').mkdir()
        self.requests = []
        requests = self.requests
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(self.path)
                self.send_response(200);self.send_header('Content-Type', 'text/html');self.end_headers()
                self.wfile.write(b'<!doctype html><title>Unit HTTP fixture</title>')
            def log_message(self, *_):pass
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=self.server.serve_forever, daemon=True);thread.start()
        self.addCleanup(lambda: (self.server.shutdown(), self.server.server_close(), thread.join()))
        self.origin = 'http://127.0.0.1:' + str(self.server.server_port)
        self.saved = []
        process = SimpleNamespace(pid=777, poll=lambda: None)
        self.qa = SimpleNamespace(process=process, url=self.origin, run_id='unit',
            http=lambda _: {'run_id': 'unit', 'token': 'SECRET_MUST_NOT_BE_PRINTED'},
            manifest=lambda: {'run_id': 'unit', 'state': 'running', 'resources': {'listeners': {
                'front.test.operator': {'host': '127.0.0.1', 'port': self.server.server_port, 'secret': 'SECRET_MUST_NOT_BE_PRINTED'}}}},
            capture=SimpleNamespace(add=lambda n, v: self.saved.append((n, v))))
        self.ap5 = SimpleNamespace(child=process, controller_ticks=123, id='unit', run=root / 'runtime', root=root / 'ap5',
            bundle={'binaries': {'lab': {'sha256': 'aa' * 32}}}, ev=SimpleNamespace(save=lambda n, v: self.saved.append((n, v))))
        self.argv = ['spc-lab', 'runtime', '--listen', '127.0.0.1:' + str(self.server.server_port), '--run-dir', str(self.ap5.run)]

    def invoke(self):
        raw = b'\0'.join(s.encode() for s in self.argv) + b'\0'
        with patch('run_info.process', return_value={'ticks': 123, 'state': 'S'}), patch('run_info.sha', return_value='aa' * 32), \
             patch.object(Path, 'read_bytes', return_value=raw), contextlib.redirect_stdout(io.StringIO()) as stdout:
            value = publish_run_info(self.qa, self.ap5)
        return value, stdout.getvalue()

    def test_prints_verified_actual_port_and_persists_only_public_fields(self):
        value, output = self.invoke()
        self.assertEqual(self.requests, ['/', '/lab'])
        self.assertEqual(value['ui'], {'controls': self.origin + '/', 'lab': self.origin + '/lab'})
        self.assertEqual(json.loads(output), value)
        self.assertEqual(json.loads(Path(value['run_info_file']).read_text()), value)
        self.assertNotIn('SECRET_MUST_NOT_BE_PRINTED', output)

    def test_foreign_snapshot_never_publishes_a_url(self):
        self.qa.http = lambda _: {'run_id': 'another-run'}
        with self.assertRaises(Blocked):self.invoke()
        self.assertFalse((self.ap5.root.parent / 'run-info.json').exists())
        self.assertEqual(self.requests, [])

    def test_changed_listen_or_run_directory_never_publishes(self):
        for name in ('--listen', '--run-dir'):
            original = list(self.argv)
            self.argv[self.argv.index(name) + 1] = '127.0.0.1:1' if name == '--listen' else '/foreign/run'
            with self.subTest(name=name), self.assertRaises(Blocked):self.invoke()
            self.argv = original
        self.assertFalse((self.ap5.root.parent / 'run-info.json').exists())

    def test_dead_runtime_never_publishes(self):
        self.qa.process.poll = lambda: 1
        with self.assertRaises(Blocked):self.invoke()
        self.assertFalse((self.ap5.root.parent / 'run-info.json').exists())
