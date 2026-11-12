"""Placement-bound read-only PostgreSQL collection for the actual lab.

The current runtime puts PostgreSQL on the host even in topology runs. Its
rendered SQLx ?host=/private/socket URL, not the unused resource TCP URL,
selects that route. A genuinely namespaced server requires the owned broker;
there is no retry through host loopback or a shared socket.
"""
from dataclasses import dataclass, field
from decimal import Decimal
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import signal
import socket
import stat
import struct
import subprocess
import tempfile
import time
import urllib.parse

from oracle import Inconclusive, Invalid, integer, require, strict_json, unhex
from storage_queries import QUERY_ID, QUERY_SHA256, ROW_CAP, TABLES, query

MAX_OUTPUT = 32 * 1024 * 1024
CONTRACT_SOURCE_FILES = ("economic_checkpoint.py", "ruleb_sql.py", "ruleb_source.py", "source_feed_codec.py",
                         "accounting_history.py", "case_contract.py", "signature_oracle.py", "public_derivation.py", "net_debits.py",
                         "decision_observations.py", "aggregate_history.py", "source_jobs.py", "pool_source.py", "ladder_wire.py")


def query_spec(contract=None):
    if contract is None:
        return query(), QUERY_ID, "spc-lab-sql-observation/2", TABLES
    from decision_observations import CONTRACT as DECISIONS, query as decision_query, layout
    if contract==DECISIONS:
        tables,_,schema,qid=layout();return decision_query(),qid,schema,tables
    from economic_checkpoint import query as checkpoint_query, QUERY_ID as qid, SCHEMA, TABLES_R3
    return checkpoint_query(source_contract=contract), qid, SCHEMA, TABLES_R3


def sha_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def text(value):
    require(isinstance(value, str) and bool(value) and not any(ord(c) < 32 or ord(c) == 127 for c in value),
            "storage string contains control/empty value")
    return value


def confined(path, root):
    path, root = Path(path), Path(root)
    require(path.is_absolute() and root.is_absolute(), "storage path must be absolute")
    require(".." not in path.parts and path.is_relative_to(root), "storage path escapes owned run")
    # Do not allow a private path to change interpretation through a symlink.
    for part in (path, *path.parents):
        if part == root.parent:
            break
        info = part.lstat()
        require(not stat.S_ISLNK(info.st_mode) and info.st_uid == os.getuid(), "storage path ownership/symlink")
    return path


def private_bytes(path, root, maximum=4 * 1024 * 1024):
    path = confined(path, root)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600,
                "storage private file mode/type")
        require(info.st_size <= maximum, "storage private file bound")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            data = stream.read(maximum + 1)
        after = os.fstat(fd)
        if (info.st_size, info.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            raise Inconclusive("storage file changed during observation")
        return data
    finally:
        os.close(fd)


def postgres_url(config):
    """Read the exact runtime-rendered [storage] JSON-compatible TOML scalar.

    Not a permissive general TOML parser: a changed producer encoding fails
    closed, rather than evaluating escapes or falling back to the resource URL.
    """
    section, values = None, []
    for line in config.splitlines():
        line = line.strip()
        if line.startswith("["):
            require(re.fullmatch(r"\[[A-Za-z0-9_.-]+\]", line) is not None, "unsupported rendered TOML section")
            section = line
        elif section == "[storage]" and re.match(r"postgres\s*=", line):
            try:
                values.append(strict_json(line.split("=", 1)[1].strip()))
            except ValueError as error:
                raise Inconclusive("unsupported rendered postgres scalar") from error
    require(len(values) == 1, "rendered postgres URL missing/duplicated")
    return text(values[0])


@dataclass(frozen=True)
class Connection:
    host: str
    port: int
    database: str
    role: str
    password: str = field(repr=False, compare=True)
    unix: bool = False

    def public(self):
        return {"host": self.host, "port": self.port, "database": self.database,
                "role": self.role, "unix": self.unix}


def parse_connection(url, database):
    text(url)
    try:
        parts = urllib.parse.urlsplit(url)
        params = urllib.parse.parse_qsl(parts.query, keep_blank_values=True, strict_parsing=True) if parts.query else []
        port = parts.port
    except ValueError as error:
        raise Invalid("invalid storage connection URL") from error
    require(parts.scheme in {"postgres", "postgresql"} and not parts.fragment,
            "storage URL scheme/fragment")
    require(all(k == "host" for k, _ in params) and len(params) <= 1,
            "unknown/duplicate storage URL options")
    require(parts.username is not None and parts.password is not None, "explicit storage credentials required")
    role = urllib.parse.unquote(parts.username, errors="strict")
    name = urllib.parse.unquote(parts.path.removeprefix("/"), errors="strict")
    password = urllib.parse.unquote(parts.password, errors="strict")
    text(role); text(name); text(password)
    require(role == database["role"] and name == database["database"] and port == database["port"],
            "rendered database/role/port differs from resource allocation")
    integer(port, 1, 65535)
    authority = text(parts.hostname)
    host = text(params[0][1]) if params else authority
    unix = host.startswith("/")
    if params:
        require(unix and authority == "localhost", "SQLx socket authority/host binding")
    if not unix:
        # Literal addresses only: no DNS or libpq multi-host routing.
        if host == "localhost":
            host = "127.0.0.1"
        try:
            ipaddress.ip_address(host)
        except ValueError as error:
            raise Invalid("storage host is not a single literal address") from error
    return Connection(host, port, name, role, password, unix)


def option(argv, flag):
    require(isinstance(argv, list) and argv.count(flag) == 1, "storage launch flag missing/duplicated: " + flag)
    index = argv.index(flag)
    require(index + 1 < len(argv), "storage launch flag has no value")
    return text(argv[index + 1])


class ProcFS:
    def fd_targets(self, pid):
        targets = set()
        for fd in Path(f"/proc/{pid}/fd").iterdir():
            try:
                targets.add(os.readlink(fd))
            except FileNotFoundError:
                # A transient non-listener descriptor may close while listed.
                # The required listener still has to be positively observed.
                continue
        return targets

    def identity(self, pid):
        integer(pid, 1, (1 << 31) - 1)
        raw = Path(f"/proc/{pid}/stat").read_text()
        tail = raw[raw.rfind(")") + 2:].split()
        require(tail[0] != "Z", "storage process is a zombie")
        return {"pid": pid, "start_time_ticks": int(tail[19]), "ppid": int(tail[1])}

    def namespace(self, pid, kind):
        info = os.stat(f"/proc/{pid}/ns/{kind}")
        return [info.st_dev, info.st_ino]

    def process(self, record):
        before = self.identity(record["pid"])
        require(before["start_time_ticks"] == record["start_time_ticks"], "storage PID/start-token mismatch")
        exe = Path(f"/proc/{record['pid']}/exe")
        require(exe.resolve() == Path(record["executable_path"]).resolve()
                and sha_file(exe) == record["executable_sha256"], "storage executable mismatch")
        env = Path(f"/proc/{record['pid']}/environ").read_bytes().split(b"\0")
        tokens = [e.split(b"=", 1)[1] for e in env if e.startswith(b"SPC_LAB_RUN_TOKEN=")]
        require(tokens == [text(record["run_token"]).encode()], "storage ownership token mismatch")
        argv = [a.decode() for a in Path(f"/proc/{record['pid']}/cmdline").read_bytes().split(b"\0") if a]
        for flag in ("-D", "-p", "-k", "-h"):
            require(option(argv, flag) == option(record["args"], flag), "storage actual argv binding")
        require(self.identity(record["pid"]) == before, "storage process changed during binding")
        return {**before, "executable_sha256": record["executable_sha256"],
                "netns": self.namespace(record["pid"], "net"),
                "userns": self.namespace(record["pid"], "user")}

    def unix_listener(self, pid, path):
        rows = Path(f"/proc/{pid}/net/unix").read_text().splitlines()[1:]
        matches = [r.split() for r in rows if len(r.split()) >= 8 and r.split()[7] == str(path)]
        require(len(matches) == 1 and matches[0][3] == "00010000", "private PostgreSQL Unix listener missing/ambiguous")
        inode = matches[0][6]
        owned = self.fd_targets(pid)
        require("socket:[" + inode + "]" in owned, "private socket is not owned by recorded postmaster")
        info = path.lstat()
        require(stat.S_ISSOCK(info.st_mode) and info.st_uid == os.getuid(), "private socket file owner/type")
        return {"filesystem_inode": info.st_ino, "socket_inode": inode}

    def tcp_listener(self, pid, address, port):
        ip = ipaddress.ip_address(address)
        require(ip.version == 4, "this source-bound storage transport supports IPv4 only")
        expected = int.from_bytes(ip.packed, "little")
        matches = []
        for row in Path(f"/proc/{pid}/net/tcp").read_text().splitlines()[1:]:
            fields = row.split()
            host, p = fields[1].split(":")
            if fields[3] == "0A" and int(host, 16) == expected and int(p, 16) == port:
                matches.append(fields[9])
        owned = self.fd_targets(pid)
        require(len(matches) == 1 and "socket:[" + matches[0] + "]" in owned,
                "declared TCP listener not owned by recorded postmaster")
        return {"socket_inode": matches[0]}


@dataclass
class StoragePlan:
    run_dir: Path
    run_id: str
    database: dict = field(repr=False)
    process_record: dict = field(repr=False)
    connection: Connection = field(repr=False)
    process_facts: dict
    controller_facts: dict
    config_bindings: list
    postmaster: dict
    route: str
    listener: dict
    broker_endpoint: str | None = None
    namespace_holder: dict | None = None
    broker_manifest_sha256: str | None = None

    def public(self):
        return {"run_id": self.run_id, "database_id": self.database["id"],
                "pool_id": self.database["owner_pool_id"], "data_directory": self.database["dir"],
                "connection": self.connection.public(), "route": self.route,
                "postmaster": self.postmaster, "process": self.process_facts,
                "controller": self.controller_facts, "rendered_config_bindings": self.config_bindings,
                "listener": self.listener, "broker_endpoint": self.broker_endpoint,
                "namespace_holder": self.namespace_holder, "broker_manifest_sha256": self.broker_manifest_sha256}


def select_route(connection, process_facts, host_netns, host_userns, database, broker, system):
    """Route from observed process placement; a topology flag selects nothing."""
    host_local = process_facts["netns"] == host_netns and process_facts["userns"] == host_userns
    if host_local:
        if connection.unix:
            require(Path(connection.host) == Path(database["dir"]) / "socket", "private socket directory differs from allocation")
            return "host_unix", None, None
        require(ipaddress.ip_address(connection.host).is_loopback, "host PostgreSQL address is not declared loopback")
        # A topology run's current runtime explicitly uses Unix sockets.
        require(broker is None, "topology PostgreSQL socket route cannot fall back to host TCP")
        return "host_tcp", None, None
    require(not connection.unix, "Unix socket cannot bypass namespaced storage isolation")
    require(broker is not None, "namespaced storage has no owned broker publication")
    endpoint_id = "storage-" + database["owner_pool_id"]
    endpoints = [e for e in broker["endpoints"] if e["name"] == endpoint_id]
    require(len(endpoints) == 1 and endpoints[0]["role"] == "storage", "pool storage endpoint missing/ambiguous")
    endpoint = endpoints[0]
    holder = endpoint["namespace"]
    facts = system.identity(holder["holder_pid"])
    require(str(facts["start_time_ticks"]) == holder["start_time_token"], "storage namespace holder start-token mismatch")
    require(holder["netns_path"] == f"/proc/{holder['holder_pid']}/ns/net", "unbound namespace path")
    require(system.namespace(holder["holder_pid"], "net") == process_facts["netns"]
            and system.namespace(holder["holder_pid"], "user") == process_facts["userns"],
            "PostgreSQL not in published storage namespace")
    addresses = {a["addr"] for a in endpoint["addresses"]}
    require(connection.host in addresses and not ipaddress.ip_address(connection.host).is_loopback,
            "namespaced PostgreSQL host must be its published storage address")
    return "namespace_tcp", endpoint_id, facts


def build_plan(run_dir, database_id, manifest, scenario, system=None):
    system = system or ProcFS()
    run_dir = Path(run_dir)
    require(manifest["resources"]["run_dir"] == str(run_dir) and manifest["run_id"] == scenario["run"]["id"],
            "storage run binding")
    dbs = [d for d in manifest["resources"]["postgres_databases"] if d["id"] == database_id]
    require(len(dbs) == 1, "database resource identity missing/duplicated")
    db = dbs[0]
    pools = [p for p in scenario["pools"] if p["id"] == db["owner_pool_id"]]
    require(len(pools) == 1 and pools[0]["storage"]["postgres_database_id"] == db["id"], "database pool ownership")
    planned = [p for p in scenario["resources"]["postgres_databases"] if p["id"] == db["id"]]
    require(len(planned) == 1 and all(planned[0][key] == db[key] for key in ("owner_pool_id", "role", "database")),
            "database differs from recorded scenario allocation")
    directory = confined(db["dir"], run_dir)
    require(stat.S_IMODE(directory.stat().st_mode) == 0o700, "PostgreSQL data directory must be private")
    processes = [p for p in manifest["processes"] if p["role"] == "postgres" and p["entity"] == db["id"]]
    require(len(processes) == 1, "postmaster process binding missing/duplicated")
    process = processes[0]
    facts = system.process(process)
    controller = system.identity(manifest["locks"]["pid"])
    require(facts["ppid"] == controller["pid"], "postmaster not a child of the owned controller")
    require(option(process["args"], "-D") == str(directory)
            and option(process["args"], "-p") == str(db["port"])
            and option(process["args"], "-k") == str(directory / "socket"), "postmaster allocation argv")
    pid_bytes = private_bytes(directory / "postmaster.pid", run_dir, 8192)
    lines = pid_bytes.decode().splitlines()
    require(len(lines) >= 6, "truncated postmaster identity file")
    require(lines[0] == str(facts["pid"]) and lines[1] == str(directory) and lines[3] == str(db["port"]),
            "postmaster file PID/directory/port mismatch")
    start_epoch = integer(int(lines[2]), 1)
    connections, config_bindings = [], []
    for front in scenario["fronts"]:
        if front["pool_id"] != db["owner_pool_id"]:
            continue
        fronts = [p for p in manifest["processes"] if p["role"] == "front" and p["entity"] == front["id"]]
        require(len(fronts) == 1, "rendered front process binding missing")
        config_path = Path(option(fronts[0]["args"], "--config"))
        require(config_path == run_dir / "private" / ("pool-" + front["id"] + ".toml"), "unexpected rendered pool config path")
        raw = private_bytes(config_path, run_dir)
        connection = parse_connection(postgres_url(raw.decode()), db)
        connections.append(connection)
        config_bindings.append({"path": str(config_path), "sha256": hashlib.sha256(raw).hexdigest()})
    require(connections and all(c == connections[0] for c in connections), "fronts disagree on actual PostgreSQL connection")
    connection = connections[0]
    # The resource URL is a credential/allocation crosscheck, never the route.
    resource_connection = parse_connection(db["url"], db)
    require(resource_connection.password == connection.password, "rendered/resource PostgreSQL credentials differ")
    broker, broker_hash = None, None
    if manifest.get("placement") is not None:
        require(manifest["placement"]["schema"] == "spc.lab.topology_placement.v1", "unknown placement schema")
        raw = private_bytes(run_dir / "broker" / "manifest.json", run_dir)
        broker, broker_hash = strict_json(raw), hashlib.sha256(raw).hexdigest()
    route, endpoint, holder = select_route(connection, facts,
                                          system.namespace(controller["pid"], "net"),
                                          system.namespace(controller["pid"], "user"), db, broker, system)
    if connection.unix:
        require(lines[4] == connection.host, "postmaster Unix socket declaration differs from rendered URL")
        socket_dir = confined(connection.host, run_dir)
        require(stat.S_IMODE(socket_dir.stat().st_mode) == 0o700, "PostgreSQL socket directory must be private")
        listener = system.unix_listener(facts["pid"], socket_dir / (".s.PGSQL." + str(connection.port)))
    else:
        require(option(process["args"], "-h") == connection.host and lines[5] == connection.host,
                "PostgreSQL TCP address differs from actual launch")
        listener = system.tcp_listener(facts["pid"], connection.host, connection.port)
    return StoragePlan(run_dir, manifest["run_id"], db, process, connection, facts, controller,
                       config_bindings, {"start_epoch": start_epoch, "identity_file_sha256": hashlib.sha256(pid_bytes).hexdigest()},
                       route, listener, endpoint, holder, broker_hash)


def conninfo(connection):
    def quoted(value):
        return "'" + text(str(value)).replace("\\", "\\\\").replace("'", "\\'") + "'"
    fields = {"host": connection.host, "port": connection.port, "dbname": connection.database,
              "user": connection.role, "connect_timeout": 5, "sslmode": "disable", "gssencmode": "disable",
              "application_name": "spc-lab-independent-oracle"}
    return " ".join(key + "=" + quoted(value) for key, value in fields.items())


def psql_command(psql, connection, query_contract=None):
    require(Path(psql["path"]).is_absolute() and sha_file(psql["path"]) == psql["sha256"], "psql executable seal mismatch")
    return [psql["path"], "--no-psqlrc", "--no-password", "--quiet", "--tuples-only", "--no-align",
            "--set", "ON_ERROR_STOP=1", "--dbname", conninfo(connection), "--command", query_spec(query_contract)[0]]


def bounded_psql(psql, connection, private_dir, query_contract=None):
    """Bound time and disk-backed output, without a shell or inherited PG env."""
    command = psql_command(psql, connection, query_contract)
    env = {"PATH": str(Path(psql["path"]).parent), "HOME": str(private_dir), "LC_ALL": "C",
           "PGPASSWORD": connection.password, "PGCLIENTENCODING": "UTF8",
           "PGOPTIONS": "-c default_transaction_read_only=on -c statement_timeout=10000 -c lock_timeout=2000"}
    # Pipes are drained with communicate into bounded regular files: no daemon
    # can deadlock on stderr; RLIMIT_FSIZE bounds both files in the child.
    import resource
    import ctypes
    parent_pid = os.getpid()
    def limit():
        if ctypes.CDLL(None).prctl(1, signal.SIGKILL, 0, 0, 0) != 0 or os.getppid() != parent_pid:
            os._exit(127)
        resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_OUTPUT, MAX_OUTPUT))
    with tempfile.TemporaryFile(dir=private_dir) as stdout, tempfile.TemporaryFile(dir=private_dir) as stderr:
        proc = subprocess.Popen(command, env=env, stdout=stdout, stderr=stderr,
                                stdin=subprocess.DEVNULL, start_new_session=True, preexec_fn=limit)
        timed_out = False
        try:
            proc.wait(timeout=18)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait(timeout=3)
        stdout.seek(0); stderr.seek(0)
        raw, errors = stdout.read(MAX_OUTPUT + 1), stderr.read(MAX_OUTPUT + 1)
    result = {"exit_code": proc.returncode, "timed_out": timed_out,
              "stdout_sha256": hashlib.sha256(raw).hexdigest(),
              "stderr_sha256": hashlib.sha256(errors).hexdigest(), "stderr_bytes": len(errors)}
    # Never expose raw libpq diagnostics: they may contain credentials.
    if timed_out or proc.returncode != 0:
        raise StorageExecutionError(result)
    require(len(raw) < MAX_OUTPUT, "psql observation output limit")
    return raw, result


class StorageExecutionError(Inconclusive):
    def __init__(self, receipt):
        super().__init__("bounded read-only psql execution failed")
        self.receipt = receipt


def validate_rows(raw, plan, query_contract=None):
    value = strict_json(raw)
    _, query_id, schema, tables = query_spec(query_contract)
    require(set(value) == {"schema", "query_id", "server", "counts", "tables", "history_boundary"}
            and value["schema"] == schema and value["query_id"] == query_id,
            "SQL observation schema/query binding")
    server = value["server"]
    require(set(server) == {"database", "role", "port", "data_directory", "unix_socket_directories",
                            "listen_addresses", "server_addr", "server_port", "postmaster_start_epoch",
                            "system_identifier", "transaction_read_only", "transaction_isolation", "backend_pid", "snapshot"},
            "SQL server metadata fields changed")
    expected = plan.connection
    require(server["database"] == expected.database and server["role"] == expected.role
            and server["port"] == expected.port and server["data_directory"] == plan.database["dir"],
            "SQL actual server resource mismatch")
    require(server["transaction_read_only"] == "on" and server["transaction_isolation"] == "repeatable read",
            "SQL transaction is not a read-only consistent snapshot")
    epoch = Decimal(server["postmaster_start_epoch"])
    require(epoch.is_finite() and int(epoch) == plan.postmaster["start_epoch"], "SQL postmaster start epoch mismatch")
    require(re.fullmatch(r"[0-9]+", server["system_identifier"]) is not None, "SQL cluster system identifier")
    require(re.fullmatch(r"[0-9]+:[0-9]+:[0-9,]*", server["snapshot"]) is not None, "SQL snapshot identity")
    integer(server["backend_pid"], 1)
    boundary = value["history_boundary"]
    require(isinstance(boundary, dict) and set(boundary) == {"wal_lsn", "event_watermark"}
            and isinstance(boundary["wal_lsn"], str) and re.fullmatch(r"[0-9A-F]+/[0-9A-F]+", boundary["wal_lsn"]),
            "SQL history snapshot/WAL boundary missing")
    if plan.route == "host_unix":
        require(server["server_addr"] is None and server["server_port"] is None
                and server["unix_socket_directories"] == expected.host, "SQL did not use the bound Unix socket")
    else:
        require(server["server_addr"] == expected.host and server["server_port"] == expected.port,
                "SQL server TCP address/port mismatch")
    require(set(value["tables"]) == set(tables) == set(value["counts"]), "SQL table coverage")
    for table, (columns, _) in tables.items():
        rows, count = value["tables"][table], integer(value["counts"][table])
        require(isinstance(rows, list), "SQL rows must be an array")
        if count > ROW_CAP or count != len(rows):
            raise Inconclusive("SQL snapshot row cap/truncation")
        for row in rows:
            require(isinstance(row, dict) and set(row) == set(columns.split()), "SQL explicit column shape changed")
    history = value["tables"]["source_accounting_history"]
    ids = [integer(row["event_id"], 1) for row in history]
    require(ids == sorted(set(ids)), "SQL history event ordering/duplicates")
    if boundary["event_watermark"] != (ids[-1] if ids else None):
        raise Inconclusive("SQL history watermark/tail mismatch")
    return value


def load_plan(run_dir, database_id, system=None):
    run_dir = Path(run_dir)
    manifest = strict_json(private_bytes(run_dir / "run-manifest.json", run_dir, 16 * 1024 * 1024))
    scenario = strict_json(private_bytes(run_dir / "private" / "scenario.json", run_dir))
    return build_plan(run_dir, database_id, manifest, scenario, system)


def observe_local(plan, psql, query_contract=None):
    """Rechecked in the selected execution namespace before and after SQL."""
    system = ProcFS()
    current = load_plan(plan.run_dir, plan.database["id"], system)
    require(current.public() == plan.public(), "storage binding changed before query")
    here_net = system.namespace(os.getpid(), "net")
    here_user = system.namespace(os.getpid(), "user")
    require(here_net == plan.process_facts["netns"] and here_user == plan.process_facts["userns"],
            "SQL collector is in the wrong storage namespace")
    raw, execution = bounded_psql(psql, current.connection, plan.run_dir / "private", query_contract)
    rows = validate_rows(raw, current, query_contract)
    after = load_plan(plan.run_dir, plan.database["id"], system)
    require(after.public() == plan.public(), "storage instance/placement changed during query")
    return {"schema": "spc-lab-bound-sql-capture/1", "binding": plan.public(),
            "query_sha256": hashlib.sha256(query_spec(query_contract)[0].encode()).hexdigest(), "psql_sha256": psql["sha256"],
            "execution": execution, "observation": rows}


def broker_exec(plan, argv, system=None):
    """Only the caller-built sealed Python/typed SQL helper argv is passed.

    SO_PEERCRED binds the endpoint to the actual run's broker process. Neither
    the broker token nor database password enters this request or its receipt.
    """
    system = system or ProcFS()
    root = plan.run_dir
    broker_dir = confined(root / "broker", root)
    require(stat.S_IMODE(broker_dir.stat().st_mode) == 0o700, "broker directory privacy")
    raw = private_bytes(broker_dir / "manifest.json", root)
    require(hashlib.sha256(raw).hexdigest() == plan.broker_manifest_sha256, "broker publication changed")
    broker = strict_json(raw)
    socket_path = confined(broker_dir / "control.sock", root)
    require(stat.S_ISSOCK(socket_path.stat().st_mode), "broker control path is not a socket")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
        stream.settimeout(23)
        stream.connect(str(socket_path))
        pid, uid, _ = struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
        require(uid == os.getuid(), "broker socket peer owner")
        peer = system.identity(pid)
        args = [a.decode() for a in Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0") if a]
        require(option(args, "--run-dir") == str(broker_dir)
                and option(args, "--run-token") == broker["run_token"], "broker peer run binding")
        require(system.namespace(pid, "user") == plan.process_facts["userns"], "broker user namespace mismatch")
        request = {"op": "exec", "endpoint": plan.broker_endpoint, "argv": argv, "timeout_s": 20}
        stream.sendall(json.dumps(request, separators=(",", ":")).encode() + b"\n")
        with stream.makefile("rb") as reader:
            response = reader.readline(MAX_OUTPUT * 3 + 1)
        require(len(response) <= MAX_OUTPUT * 3 and response.endswith(b"\n"), "broker SQL response bound/tail")
        require(system.identity(pid) == peer, "broker changed during SQL execution")
    result = strict_json(response)
    metadata = {"broker_pid": pid, "broker_start_time_ticks": peer["start_time_ticks"],
                "command_exit_code": result.get("command_exit_code"), "endpoint": plan.broker_endpoint,
                "response_sha256": hashlib.sha256(response).hexdigest()}
    if result.get("error") is not None or result.get("command_exit_code") != 0:
        raise StorageExecutionError(metadata)
    if result.get("stdout_truncated") or result.get("stderr_truncated") or result.get("timed_out"):
        raise StorageExecutionError(metadata)
    require(isinstance(result.get("stdout"), str), "broker SQL stdout missing")
    helper = strict_json(result["stdout"])
    require(helper.get("status") == "PASSED" and set(helper) == {"status", "capture"}, "typed SQL helper did not complete")
    return helper["capture"], metadata


def collect_database(run_dir, database_id, psql, python, postgres, private_output, observer=None, query_contract=None):
    """Production entry point: no injected values/SQL or fixture transport."""
    plan = load_plan(run_dir, database_id)
    sql, query_id, _, _ = query_spec(query_contract)
    query_hash = hashlib.sha256(sql.encode()).hexdigest()
    require(Path(plan.process_record["executable_path"]).resolve() == Path(postgres["path"]).resolve()
            and plan.process_facts["executable_sha256"] == postgres["sha256"], "postmaster differs from explicit sealed binary")
    require(sha_file(psql["path"]) == psql["sha256"], "collector psql seal")
    if observer:
        observer("sql_binding", {"binding": plan.public(), "query_id": query_id, "query_sha256": query_hash})
    try:
        if plan.route != "namespace_tcp":
            result = observe_local(plan, psql, query_contract)
        else:
            require(sha_file(python["path"]) == python["sha256"], "SQL helper interpreter seal")
            helper = Path(__file__).resolve().with_name("storage_exec.py")
            request = {"schema": "spc-lab-sql-helper-request/1", "run_dir": str(plan.run_dir),
                       "database_id": database_id, "binding": plan.public(), "query_sha256": query_hash,
                       "psql": {"path": psql["path"], "sha256": psql["sha256"]},
                       "helper_sha256": sha_file(helper), "storage_sha256": sha_file(__file__),
                       "oracle_sha256": sha_file(Path(__file__).with_name("oracle.py")),
                        "queries_source_sha256": sha_file(Path(__file__).with_name("storage_queries.py"))}
            if query_contract is not None:
                request["query_contract"] = query_contract
                request["contract_sources"] = {name: sha_file(Path(__file__).with_name(name)) for name in CONTRACT_SOURCE_FILES}
            request_dir = plan.run_dir / "private" / "acceptance-sql"
            request_dir.mkdir(mode=0o700, exist_ok=True)
            confined(request_dir, plan.run_dir)
            require(stat.S_IMODE(request_dir.stat().st_mode) == 0o700, "SQL helper request directory privacy")
            # This stores paths/hashes/placement only, never a password/token.
            fd, name = tempfile.mkstemp(prefix="request-", suffix=".json", dir=request_dir)
            with os.fdopen(fd, "wb") as stream:
                stream.write(json.dumps(request, sort_keys=True).encode())
                stream.flush(); os.fsync(stream.fileno())
            result, execution = broker_exec(plan, [python["path"], "-B", str(helper), "--request", name])
            if observer:
                observer("sql_broker_exit", execution)
        require(result["schema"] == "spc-lab-bound-sql-capture/1" and result["binding"] == plan.public()
                and result["query_sha256"] == query_hash and result["psql_sha256"] == psql["sha256"],
                "SQL capture result binding")
        # The outer controller independently checks typed helper output too.
        validate_rows(json.dumps(result["observation"], default=str).encode(), plan, query_contract)
        require(load_plan(run_dir, database_id).public() == plan.public(), "storage changed after collection")
        path = Path(private_output)
        encoded = json.dumps(result, sort_keys=True, default=str).encode() + b"\n"
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(encoded); stream.flush(); os.fsync(stream.fileno())
        summary = {"database_id": database_id, "pool_id": plan.database["owner_pool_id"], "route": plan.route,
                   "query_sha256": query_hash, "capture_sha256": hashlib.sha256(encoded).hexdigest(),
                    "row_counts": result["observation"]["counts"], "execution": result["execution"]}
        from accounting_history import replay
        try:
            if query_contract is not None:
                raise Inconclusive("source-r3 checkpoint requires parent-bound seed/RuleB/actual-E replay by collector")
            history = replay(result["observation"])
            summary["accounting_history"] = {"status": "PASSED_ROW_REPLAY_ONLY", "events": history["events"],
                                              "event_watermark": history["event_watermark"],
                                              "committed_transactions": len(history["transactions"]),
                                              "past_job_pending_verified": False, "reservation_history_complete": False}
        except Inconclusive as error:
            summary["accounting_history"] = {"status": "INCONCLUSIVE", "reason": str(error)}
        if observer:
            observer("sql_capture_complete", summary)
        return summary
    except StorageExecutionError as error:
        if observer:
            observer("sql_capture_incomplete", error.receipt)
        raise
