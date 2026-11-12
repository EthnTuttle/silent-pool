"""Offline transport negatives in actual producer formats; no funded proof."""
import copy
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import unittest
from unittest.mock import patch

from oracle import Inconclusive, Invalid, require
from storage import (Connection, ProcFS, StorageExecutionError, bounded_psql,
                     build_plan, collect_database, conninfo, parse_connection, postgres_url,
                     psql_command, select_route, sha_file, validate_rows)
from storage_queries import QUERY_ID, QUERY_SHA256, ROW_CAP, TABLES, query

TEMP_BASE = None
SECRET = "test-only/pg'password"


class FixtureProcFS:
    """Only injected into unit-level build_plan; no production CLI selects it."""
    def __init__(self, namespaced=False):
        self.facts = {"pid": 100, "start_time_ticks": 123, "ppid": 50,
                      "executable_sha256": "ab" * 32,
                      "netns": [1, 20 if namespaced else 10], "userns": [1, 40 if namespaced else 30]}
        self.holder_ticks = 555

    def identity(self, pid):
        return {"pid": pid, "start_time_ticks": self.holder_ticks if pid == 200 else (123 if pid == 100 else 222), "ppid": 50 if pid == 100 else 1}

    def process(self, record):
        require(record["pid"] == 100 and record["start_time_ticks"] == self.facts["start_time_ticks"], "storage PID/start-token mismatch")
        return dict(self.facts)

    def namespace(self, pid, kind):
        if pid in (100, 200):
            return self.facts["netns" if kind == "net" else "userns"]
        return [1, 10 if kind == "net" else 30]

    def unix_listener(self, pid, path):
        return {"filesystem_inode": 999, "socket_inode": "888"}

    def tcp_listener(self, pid, address, port):
        return {"socket_inode": "777"}


class StorageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="t3-sql-vectors-", dir=TEMP_BASE)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.root.chmod(0o700)
        for path in (self.root / "private", self.root / "pg", self.root / "pg" / "socket", self.root / "broker"):
            path.mkdir(mode=0o700)
        self.proc = FixtureProcFS()
        self.db = {"id": "pg-a", "owner_pool_id": "pool-a", "database": "spc_a", "role": "spc_a",
                   "port": 43210, "dir": str(self.root / "pg"),
                   "url": "postgres://spc_a:test-only%2Fpg%27password@127.0.0.1:43210/spc_a"}
        self.config = self.root / "private" / "pool-front-a.toml"
        self.url = "postgres://spc_a:test-only%2Fpg%27password@localhost:43210/spc_a?host=" + str(self.root / "pg" / "socket")
        self.write(self.config, '[pool]\nnetwork = "regtest"\n[storage]\npostgres = ' + json.dumps(self.url) + '\nredis = "not-used"\n')
        self.pidfile = self.root / "pg" / "postmaster.pid"
        self.write(self.pidfile, f"100\n{self.root / 'pg'}\n1700000000\n43210\n{self.root / 'pg' / 'socket'}\n127.0.0.1\n0 0\nready\n")
        self.pg = {"role": "postgres", "entity": "pg-a", "pid": 100, "start_time_ticks": 123,
                   "executable_path": "/sealed/postgres", "executable_sha256": "ab" * 32, "run_token": "private-fixture-token",
                   "args": ["-D", str(self.root / "pg"), "-p", "43210", "-h", "127.0.0.1", "-k", str(self.root / "pg" / "socket")]}
        self.manifest = {"run_id": "run", "resources": {"run_dir": str(self.root), "postgres_databases": [self.db]},
                         "processes": [self.pg, {"role": "front", "entity": "front-a", "args": ["--config", str(self.config)]}],
                         "locks": {"pid": 50}, "placement": {"schema": "spc.lab.topology_placement.v1"}}
        self.scenario = {"run": {"id": "run"}, "pools": [{"id": "pool-a", "storage": {"postgres_database_id": "pg-a"}}],
                         "fronts": [{"id": "front-a", "pool_id": "pool-a"}],
                         "resources": {"postgres_databases": [{k: self.db[k] for k in ("id", "owner_pool_id", "database", "role")} ]}}
        self.broker = {"run_id": "broker-run", "run_token": "private-broker-token", "endpoints": [{"name": "storage-pool-a", "role": "storage",
                       "namespace": {"holder_pid": 200, "start_time_token": "555", "netns_path": "/proc/200/ns/net"},
                       "addresses": [{"addr": "10.99.1.2", "link_id": "front-a--storage", "iface": "veth0"}]}]}
        self.write(self.root / "broker" / "manifest.json", json.dumps(self.broker))

    def write(self, path, value):
        path.write_text(value)
        path.chmod(0o600)

    def plan(self):
        return build_plan(self.root, "pg-a", self.manifest, self.scenario, self.proc)

    def namespace_fixture(self):
        self.proc = FixtureProcFS(namespaced=True)
        self.url = "postgres://spc_a:test-only%2Fpg%27password@10.99.1.2:43210/spc_a"
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.url) + '\n')
        self.pg["args"][self.pg["args"].index("-h") + 1] = "10.99.1.2"
        self.write(self.pidfile, self.pidfile.read_text().replace("127.0.0.1", "10.99.1.2"))

    def observation(self, plan):
        return {"schema": "spc-lab-sql-observation/2", "query_id": QUERY_ID,
                "history_boundary": {"wal_lsn": "0/100", "event_watermark": None},
                "server": {"database": "spc_a", "role": "spc_a", "port": 43210, "data_directory": str(self.root / "pg"),
                           "unix_socket_directories": str(self.root / "pg" / "socket"), "listen_addresses": "127.0.0.1",
                           "server_addr": None if plan.connection.unix else plan.connection.host,
                           "server_port": None if plan.connection.unix else 43210,
                           "postmaster_start_epoch": "1700000000.123456", "system_identifier": "99999999999999",
                           "transaction_read_only": "on", "transaction_isolation": "repeatable read", "backend_pid": 101, "snapshot": "4:9:5,7"},
                "counts": {table: 0 for table in TABLES}, "tables": {table: [] for table in TABLES}}

    def test_topology_postgres_is_host_private_unix_not_redis_namespace(self):
        plan = self.plan()
        self.assertEqual(plan.route, "host_unix")
        self.assertEqual(plan.connection.host, str(self.root / "pg" / "socket"))
        self.assertIsNone(plan.broker_endpoint)
        self.assertEqual(plan.connection.password, SECRET)
        self.assertNotIn(SECRET, json.dumps(plan.public()))
        self.assertNotIn("private-fixture-token", json.dumps(plan.public()))
        self.assertNotIn("password", repr(plan))

    def test_resource_tcp_url_never_overrides_actual_rendered_socket(self):
        self.db["url"] = self.db["url"].replace("127.0.0.1", "10.200.30.1")
        self.assertEqual(self.plan().route, "host_unix")

    def test_host_tcp_without_topology_keeps_declared_address(self):
        self.manifest["placement"] = None
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.db["url"]) + '\n')
        self.assertEqual(self.plan().route, "host_tcp")

    def test_topology_host_tcp_fallback_refused(self):
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.db["url"]) + '\n')
        with self.assertRaisesRegex(Invalid, "cannot fall back"):
            self.plan()

    def test_namespace_transport_is_broker_only(self):
        self.namespace_fixture()
        plan = self.plan()
        self.assertEqual(plan.route, "namespace_tcp")
        self.assertEqual(plan.broker_endpoint, "storage-pool-a")
        self.assertEqual(plan.namespace_holder["start_time_ticks"], 555)

    def test_namespace_socket_bypass_refused(self):
        self.proc = FixtureProcFS(namespaced=True)
        with self.assertRaisesRegex(Invalid, "cannot bypass"):
            self.plan()

    def test_namespace_host_loopback_not_aliased(self):
        self.namespace_fixture()
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.db["url"]) + '\n')
        with self.assertRaisesRegex(Invalid, "published storage address"):
            self.plan()

    def test_namespace_holder_reuse_and_wrong_address(self):
        self.namespace_fixture()
        self.proc.holder_ticks += 1
        with self.assertRaisesRegex(Invalid, "holder start-token"):
            self.plan()
        self.proc.holder_ticks -= 1
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.url.replace("10.99.1.2", "10.99.1.3")) + '\n')
        with self.assertRaisesRegex(Invalid, "published storage address"):
            self.plan()

    def test_process_reuse_refused(self):
        self.pg["start_time_ticks"] += 1
        with self.assertRaisesRegex(Invalid, "PID/start-token"):
            self.plan()

    def test_wrong_postmaster_pid_or_port_refused(self):
        for old, new in (("100\n", "101\n"), ("43210\n", "43211\n")):
            original = self.pidfile.read_text()
            self.write(self.pidfile, original.replace(old, new, 1))
            with self.assertRaisesRegex(Invalid, "file PID/directory/port"):
                self.plan()
            self.write(self.pidfile, original)

    def test_resource_database_and_config_password_mismatch(self):
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.url.replace("test-only%2Fpg%27password", "another-password")) + '\n')
        with self.assertRaisesRegex(Invalid, "credentials differ"):
            self.plan()

    def test_no_symlink_or_foreign_socket_directory(self):
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.url.replace(str(self.root / "pg" / "socket"), "/tmp/foreign")) + '\n')
        with self.assertRaisesRegex(Invalid, "socket directory differs"):
            self.plan()
        self.write(self.config, '[storage]\npostgres = ' + json.dumps(self.url) + '\n')
        moved = self.config.with_suffix(".old")
        self.config.rename(moved)
        self.config.symlink_to(moved)
        with self.assertRaisesRegex(Invalid, "symlink"):
            self.plan()

    def test_duplicate_unknown_url_options_fail_before_transport(self):
        for suffix in ("&host=/other", "&hostaddr=127.0.0.1", "&options=-cfoo", "&service=foreign"):
            with self.assertRaises(Invalid):
                parse_connection(self.url + suffix, self.db)

    def test_url_role_port_database_and_mixed_rendering_refused(self):
        for url in (self.url.replace(":43210/", ":5432/"), self.url.replace("/spc_a?", "/foreign?"),
                    self.url.replace("//spc_a:", "//other:")):
            with self.assertRaises(Invalid):
                parse_connection(url, self.db)
        with self.assertRaises(Invalid):
            postgres_url('[storage]\npostgres="a"\npostgres="b"\n')

    def test_conninfo_quotes_values_without_password_or_sql_injection(self):
        c = Connection("127.0.0.1", 43210, "db' host=evil", "user\\name", SECRET)
        encoded = conninfo(c)
        self.assertIn("dbname='db\\' host=evil'", encoded)
        self.assertIn("user='user\\\\name'", encoded)
        self.assertNotIn(SECRET, encoded)
        cmd = psql_command({"path": sys.executable, "sha256": sha_file(sys.executable)}, c)
        self.assertEqual(cmd[-1], query())
        self.assertNotIn(c.database, query())
        self.assertNotIn("SELECT *", query())
        self.assertIn("REPEATABLE READ READ ONLY", query())

    def test_readonly_result_and_actual_row_shape(self):
        plan = self.plan()
        data = self.observation(plan)
        row = {"sp_id": "\\x" + "11" * 32, "sats": 9007199254740993, "updated_at": "2026-11-09T00:00:00Z"}
        data["tables"]["pending_balance"] = [row]
        data["counts"]["pending_balance"] = 1
        actual = validate_rows(json.dumps(data).encode(), plan)
        self.assertEqual(actual["tables"]["pending_balance"][0]["sats"], 9007199254740993)

    def test_actual_server_socket_vs_tcp_and_cluster_epoch_negatives(self):
        plan = self.plan()
        for key, value in (("server_addr", "127.0.0.1"), ("port", 5432), ("postmaster_start_epoch", "1700000001.0"),
                           ("transaction_read_only", "off"), ("transaction_isolation", "read committed"), ("role", "other")):
            data = self.observation(plan)
            data["server"][key] = value
            with self.subTest(key=key), self.assertRaises(Invalid):
                validate_rows(json.dumps(data).encode(), plan)

    def test_namespace_result_must_match_exact_declared_address(self):
        self.namespace_fixture()
        plan = self.plan()
        data = self.observation(plan)
        validate_rows(json.dumps(data).encode(), plan)
        data["server"]["server_addr"] = "127.0.0.1"
        with self.assertRaises(Invalid):
            validate_rows(json.dumps(data).encode(), plan)

    def test_missing_and_truncated_rows_never_zero_filled(self):
        plan = self.plan()
        data = self.observation(plan)
        del data["tables"]["slice_credit"]
        with self.assertRaises(Invalid):
            validate_rows(json.dumps(data).encode(), plan)
        data = self.observation(plan)
        data["counts"]["slice"] = 1
        with self.assertRaises(Inconclusive):
            validate_rows(json.dumps(data).encode(), plan)
        data["counts"]["slice"] = ROW_CAP + 1
        with self.assertRaises(Inconclusive):
            validate_rows(json.dumps(data).encode(), plan)

    def test_collected_history_tail_is_bound_to_same_snapshot_watermark(self):
        plan = self.plan()
        data = self.observation(plan)
        row = {"event_id": 4, "transaction_id": 3, "table_name": "checkpoint", "operation": "history_start",
               "before_row": None, "after_row": {}, "recorded_at": "now"}
        data["tables"]["source_accounting_history"] = [row]
        data["counts"]["source_accounting_history"] = 1
        data["history_boundary"]["event_watermark"] = 5
        with self.assertRaisesRegex(Inconclusive, "watermark/tail"):
            validate_rows(json.dumps(data).encode(), plan)
        data["history_boundary"]["event_watermark"] = 4
        validate_rows(json.dumps(data).encode(), plan)

    def test_psql_failure_redacts_credentials_and_drops_inherited_pg_env(self):
        calls = []
        class FailedPsql:
            pid, returncode = 99999, 2
            def __init__(self, command, **kwargs):
                calls.append((command, kwargs))
                kwargs["stderr"].write(("password=" + SECRET).encode())
            def wait(self, timeout):
                return self.returncode
        with patch.dict(os.environ, {"PGOPTIONS": "untrusted", "PGHOSTADDR": "evil"}), patch("storage.subprocess.Popen", FailedPsql):
            with self.assertRaises(StorageExecutionError) as caught:
                bounded_psql({"path": sys.executable, "sha256": sha_file(sys.executable)}, self.plan().connection, self.root / "private")
        self.assertEqual(calls[0][1]["env"]["PGPASSWORD"], SECRET)
        self.assertNotIn("PGHOSTADDR", calls[0][1]["env"])
        self.assertNotIn("untrusted", calls[0][1]["env"]["PGOPTIONS"])
        self.assertNotIn(SECRET, json.dumps(caught.exception.receipt))
        self.assertNotIn(SECRET, repr(calls[0][0]))

    def test_real_kernel_unix_listener_is_bound_to_owner(self):
        path = self.root / "pg" / "socket" / ".s.PGSQL.43210"
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(path)); listener.listen(1)
            proof = ProcFS().unix_listener(os.getpid(), path)
            self.assertIn("socket_inode", proof)
            with self.assertRaises(Invalid):
                ProcFS().unix_listener(os.getpid(), path.with_name(".s.PGSQL.43211"))

    def test_namespace_broker_failure_never_falls_back_to_host_psql(self):
        self.namespace_fixture()
        plan = self.plan()
        executable = {"path": sys.executable, "sha256": sha_file(sys.executable)}
        postgres = {"path": self.pg["executable_path"], "sha256": self.pg["executable_sha256"]}
        with patch("storage.load_plan", return_value=plan), patch("storage.observe_local") as local, \
             patch("storage.broker_exec", side_effect=StorageExecutionError({"command_exit_code": 2})) as broker:
            with self.assertRaises(StorageExecutionError):
                collect_database(self.root, "pg-a", executable, executable, postgres, self.root / "private" / "result.json")
        local.assert_not_called()
        broker.assert_called_once()
        self.assertFalse((self.root / "private" / "result.json").exists())
        requests = list((self.root / "private" / "acceptance-sql").glob("*.json"))
        self.assertEqual(len(requests), 1)
        self.assertNotIn(SECRET, requests[0].read_text())
        self.assertNotIn("test-only%2Fpg%27password", requests[0].read_text())

    def test_typed_helper_refuses_caller_sql_before_execution(self):
        from storage_exec import execute_request
        directory = self.root / "private" / "acceptance-sql"
        directory.mkdir(mode=0o700)
        path = directory / "request.json"
        self.write(path, json.dumps({"schema": "spc-lab-sql-helper-request/1", "sql": "DELETE FROM ledger_history"}))
        with self.assertRaisesRegex(Invalid, "SQL helper request fields"):
            execute_request(path)


if __name__ == "__main__":
    unittest.main()
