#!/usr/bin/env python3
"""Shared, fail-closed primitives for the independent v0.9 release validator."""

from __future__ import annotations

import dataclasses
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import signal
import stat
import subprocess
import time
from typing import Any

HEX64 = re.compile(r"[0-9a-f]{64}")
RUN_PREFIX = Path("/tmp/opencode/LP-v09-live-")
RUN_PREFIXES = (RUN_PREFIX, Path("/tmp/opencode/LP-v09-r2-"))
TOKEN_VARIABLE = "SPC_V09_OWNED_RUN_TOKEN"


class ValidationError(Exception):
    def __init__(self, code: str, message: str, outcome: str = "failed"):
        super().__init__(message)
        if outcome not in {"failed", "unknown", "not_run"}:
            raise ValueError("invalid validation outcome")
        self.code = code
        self.message = message
        self.outcome = outcome

    def as_dict(self) -> dict:
        return {"code": self.code, "message": self.message, "outcome": self.outcome}


def require(condition: bool, code: str, message: str, outcome: str = "failed") -> None:
    if not condition:
        raise ValidationError(code, message, outcome)


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_bytes(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as source:
        return json.load(source)


def atomic_write(path: Path, content: Any, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if isinstance(content, (dict, list)):
        content = json.dumps(content, indent=2, sort_keys=True) + "\n"
    if isinstance(content, str):
        content = content.encode("utf-8")
    temporary = path.with_name(f".{path.name}.{os.getpid()}.{secrets.token_hex(4)}.tmp")
    try:
        with temporary.open("xb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def absolute_file(value: Any, field: str, executable: bool = False) -> Path:
    require(isinstance(value, str) and value, "input_missing", f"{field} is required", "not_run")
    path = Path(value)
    require(path.is_absolute(), "input_path", f"{field} must be absolute")
    require(path.is_file() and not path.is_symlink(), "input_missing", f"{field} is not a regular file: {path}", "not_run")
    if executable:
        require(os.access(path, os.X_OK), "binary_not_executable", f"{field} is not executable: {path}")
    return path.resolve()


def absolute_dir(value: Any, field: str) -> Path:
    require(isinstance(value, str) and value, "input_missing", f"{field} is required", "not_run")
    path = Path(value)
    require(path.is_absolute(), "input_path", f"{field} must be absolute")
    require(path.is_dir() and not path.is_symlink(), "input_missing", f"{field} is not a directory: {path}", "not_run")
    return path.resolve()


def full_hash(value: Any, field: str) -> str:
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None,
            "hash_shape", f"{field} must be a full lowercase SHA-256")
    return value


def safe_member(root: Path, relative: str) -> Path:
    member = Path(relative)
    require(not member.is_absolute() and ".." not in member.parts and relative not in {"", "."},
            "manifest_path", f"unsafe manifest path: {relative}")
    path = root / member
    require(not path.is_symlink() and path.is_file(), "manifest_path", f"manifest member is not a regular non-symlink: {relative}")
    resolved = path.resolve()
    require(resolved.is_relative_to(root.resolve()), "manifest_path", f"manifest member escapes source: {relative}")
    return path


def canonical_snapshot(manifest: dict, subset: str = "path_sha256") -> str:
    require(isinstance(manifest.get("base"), str) and manifest["base"], "manifest_shape", "manifest base is missing")
    rows = manifest.get("files")
    require(isinstance(rows, list) and rows, "manifest_shape", "manifest files must be a nonempty array")
    files = []
    for row in rows:
        require(isinstance(row, dict), "manifest_shape", "manifest file row is not an object")
        relative = row.get("path")
        require(isinstance(relative, str) and re.fullmatch(r"[0-9a-f]{64}", row.get("sha256", "")) is not None,
                "manifest_shape", f"invalid manifest row: {relative!r}")
        if subset in {"runtime_code", "runtime_full"}:
            require(re.fullmatch(r"100(?:644|755)", row.get("mode", "")) is not None,
                    "manifest_shape", f"runtime manifest needs a regular Git mode for {relative}")
            if subset == "runtime_code" and relative == manifest.get("report_path"):
                continue
            files.append({"mode": row["mode"], "path": relative, "sha256": row["sha256"]})
        else:
            files.append({"path": relative, "sha256": row["sha256"]})
    paths = [row["path"] for row in files]
    require(len(paths) == len(set(paths)), "manifest_duplicate", "duplicate manifest path")
    files.sort(key=lambda row: row["path"])
    encoded = json.dumps({"base": manifest["base"], "files": files}, sort_keys=True, separators=(",", ":")).encode()
    return sha256_bytes(encoded)


def verify_source_manifest(source_dir: Path, manifest_path: Path, expected_snapshot: str,
                           subset: str = "path_sha256") -> dict:
    manifest = load_json(manifest_path)
    require(isinstance(manifest, dict), "manifest_shape", "manifest root must be an object")
    actual = canonical_snapshot(manifest, subset)
    expected_snapshot = full_hash(expected_snapshot, "expected source snapshot")
    require(actual == expected_snapshot, "manifest_snapshot", f"snapshot mismatch: expected {expected_snapshot}, computed {actual}")
    if subset == "path_sha256":
        require(manifest.get("snapshot_sha256") == actual, "manifest_self_hash", "manifest snapshot_sha256 does not match its recipe")
    elif subset == "runtime_code":
        require(manifest.get("code_snapshot_sha256") == actual, "manifest_self_hash", "runtime code snapshot does not match its recipe")
    elif subset == "runtime_full":
        require(manifest.get("frozen_source_snapshot_sha256") == actual, "manifest_self_hash", "runtime full snapshot does not match its recipe")
    else:
        raise ValidationError("manifest_recipe", f"unknown manifest recipe {subset}")
    source_root = source_dir.resolve()
    recorded_root = manifest.get("source_root")
    if recorded_root is not None:
        require(Path(recorded_root) == source_root, "manifest_root", f"manifest source_root {recorded_root} does not match {source_root}")
    original_paths = [row.get("path") for row in manifest["files"]]
    require(original_paths == sorted(original_paths), "manifest_order", "manifest file rows must be sorted by path")
    report_path = manifest.get("report_path")
    for row in manifest["files"]:
        member = safe_member(source_root, row["path"])
        require(sha256_file(member) == row["sha256"], "source_drift", f"source changed: {row['path']}")
        if subset.startswith("runtime_"):
            mode = "100755" if os.access(member, os.X_OK) else "100644"
            require(mode == row["mode"], "source_mode", f"mode changed: {row['path']}")
        if report_path is not None and subset == "runtime_code":
            continue
    return {
        "source_dir": str(source_root), "manifest_path": str(manifest_path),
        "manifest_sha256": sha256_file(manifest_path), "snapshot_sha256": actual,
        "file_count": len(manifest["files"]), "recipe": subset, "verified": True,
    }


def _pass_sections(text: str) -> list[str]:
    lines = text.splitlines()
    starts = []
    fenced = False
    for index, line in enumerate(lines):
        if line.startswith("```"):
            fenced = not fenced
            continue
        if not fenced and re.match(r"^# [^#]", line):
            starts.append(index)
    if not starts or starts[0] != 0:
        starts.insert(0, 0)
    starts.append(len(lines))
    sections = []
    for begin, end in zip(starts, starts[1:]):
        body = "\n".join(lines[begin:end])
        if re.search(r"(?m)^## (?:Verdict|Result|Judgment)(?:\s|\(|$)", body):
            sections.append(body)
    return sections


def _unfenced_lines(body: str) -> list[tuple[int, str]]:
    """Return verdict/prose lines outside fenced examples, with original indices."""
    lines = []
    fenced = False
    for index, line in enumerate(body.splitlines()):
        if line.startswith("```"):
            fenced = not fenced
            continue
        if not fenced:
            lines.append((index, line))
    return lines


def _verdict_entries(body: str) -> list[tuple[str, int]]:
    entries = []
    for lineno, line in _unfenced_lines(body):
        stripped = line.strip()
        match = re.match(r"^\*\*(APPROVE|REQUEST_CHANGES)(?:\b[^*]*)?\*\*", stripped)
        if not match:
            match = re.match(r"^(APPROVE|REQUEST_CHANGES)\s+on\s+snapshot\b", stripped)
        if match:
            entries.append((match.group(1), lineno))
    return entries


def _approval_pass_facts(body: str) -> dict:
    """Parse one candidate set per pass; never associate one hash at a time.

    A full `tied to snapshot` is itself a declaration. Short prefixes filter
    candidates, but do not manufacture a missing full hash. The one compound
    runtime binding is kept as an explicit code/full pair, not two unrelated
    independent candidates.
    """
    lines = _unfenced_lines(body)
    clean = "\n".join(line for _, line in lines)
    tied = re.findall(r"tied to snapshot\s+`?([0-9a-f]{7,64})(?:\.\.\.([0-9a-f]{4,64}))?", clean)
    full_candidates = set(re.findall(r"snapshot_sha256[`\s]*([0-9a-f]{64})", clean))
    full_candidates.update(prefix for prefix, _ in tied if len(prefix) == 64)
    full_candidates.update(re.findall(r"(?m)^([0-9a-f]{64})\s+\(matches\)\s*$", body))
    for index, line in lines:
        if re.match(r"(?i)^snapshot under review:\s*`?[0-9a-f]{64}`?\s*$", line):
            full_candidates.add(re.search(r"[0-9a-f]{64}", line).group(0))
        elif re.match(r"(?i)^snapshot under review:\s*$", line):
            following = [value for number, value in lines if number == index + 1]
            if following and re.fullmatch(r"`?([0-9a-f]{64})`?", following[0].strip()):
                full_candidates.add(following[0].strip().strip("`"))
    for match in re.finditer(r"(?:APPROVE|REQUEST_CHANGES)\s+on\s+snapshot\s*\n\s*`?([0-9a-f]{64})(?![0-9a-f])", clean):
        full_candidates.add(match.group(1))
    runtime_pairs = []
    for match in re.finditer(
            r"(?mi)^This approval binds code aggregate\s+`?([0-9a-f]{64})`?[^\n]*?"
            r"\bfull\s+`([0-9a-f]{64})`", clean):
        pair = {"code_snapshot_sha256": match.group(1), "full_snapshot_sha256": match.group(2)}
        if pair not in runtime_pairs:
            runtime_pairs.append(pair)
    return {
        "verdicts": _verdict_entries(body), "tied": tied,
        "full_candidates": full_candidates, "runtime_pairs": runtime_pairs,
    }


def _prefix_matches(snapshot: str, tied: list[tuple[str, str]]) -> bool:
    return all(snapshot.startswith(prefix) and (not suffix or snapshot.endswith(suffix))
               for prefix, suffix in tied)


def approval_decision(text: str, snapshot: str) -> dict:
    """Bind a single unambiguous full hash per APPROVE pass, with broad vetoes.

    Conflicting full declarations invalidate the whole pass for approval.
    REQUEST_CHANGES remains a veto for every full hash or tied prefix it names.
    Fenced examples, unrelated bare hashes, and new unsolicited formats never
    become approvals.
    """
    full_hash(snapshot, "approval snapshot")
    approvals = []
    rejections = []
    for section in _pass_sections(text):
        facts = _approval_pass_facts(section)
        kinds = {kind for kind, _ in facts["verdicts"]}
        tied_match = _prefix_matches(snapshot, facts["tied"])
        if "REQUEST_CHANGES" in kinds:
            pair_hit = any(snapshot in pair.values() for pair in facts["runtime_pairs"])
            if tied_match and (snapshot in facts["full_candidates"] or pair_hit or facts["tied"]):
                heading = section.splitlines()[0]
                if heading not in rejections:
                    rejections.append(heading)
        if "APPROVE" not in kinds or not tied_match:
            continue
        # A compound runtime pair is one binding. Mixing it with another
        # independent candidate, or multiple pairs, is ambiguous.
        if facts["runtime_pairs"]:
            if len(facts["runtime_pairs"]) != 1 or facts["full_candidates"]:
                continue
            approved_here = snapshot in facts["runtime_pairs"][0].values()
        else:
            approved_here = len(facts["full_candidates"]) == 1 and snapshot in facts["full_candidates"]
        if approved_here:
            heading = section.splitlines()[0]
            if heading not in approvals:
                approvals.append(heading)
    return {
        "approved": bool(approvals) and not rejections,
        "approval_sections": approvals,
        "permanent_rejection_sections": rejections,
    }


def verify_approval(path: Path, snapshot: str, component: str, required: bool) -> dict:
    if not required and not path.exists():
        return {"required": False, "status": "not_run", "reason": "diagnostic mode does not require approval"}
    text = path.read_text(encoding="utf-8", errors="strict")
    decision = approval_decision(text, snapshot)
    require(decision["approved"], "approval_mismatch",
            f"{component}: no unrevoked, pass-local, full-snapshot APPROVE for {snapshot} in {path}")
    return {"required": required, "status": "approved", "approval_record": str(path),
            "approval_record_sha256": sha256_file(path), **decision}


def verify_binary(path: Path, expected_sha256: str) -> dict:
    expected_sha256 = full_hash(expected_sha256, "binary SHA-256")
    info = path.stat()
    actual = sha256_file(path)
    require(actual == expected_sha256, "binary_hash", f"binary hash mismatch for {path}: expected {expected_sha256}, got {actual}")
    return {"path": str(path), "sha256": actual, "size": info.st_size, "mtime_ns": info.st_mtime_ns}


def verify_nix_provenance(path: Path, expected_names: set[str], query_store: bool = True) -> dict:
    rows = load_json(path)
    require(isinstance(rows, list) and rows, "nix_provenance", "Nix provenance must be a nonempty array")
    records = {}
    for row in rows:
        require(isinstance(row, dict) and row.get("name") not in records, "nix_provenance", "duplicate or malformed Nix record")
        name = row.get("name")
        require(name in expected_names, "nix_provenance", f"unexpected Nix component {name}")
        for field in ["binary", "output", "derivation", "sha256"]:
            require(isinstance(row.get(field), str) and row[field], "nix_provenance", f"{name} lacks {field}")
        for field in ["binary", "output", "derivation"]:
            require(Path(row[field]).is_absolute() and str(Path(row[field])).startswith("/nix/store/"),
                    "nix_provenance", f"{name} {field} is outside immutable /nix/store")
        binary = absolute_file(row["binary"], f"{name}.binary", executable=True)
        output = Path(row["output"]).resolve()
        require(binary.is_relative_to(output), "nix_provenance", f"{name} binary is outside its declared output")
        record = verify_binary(binary, row["sha256"])
        record.update({"output": str(output), "derivation": row["derivation"]})
        if query_store:
            queried = subprocess.run(["nix-store", "--query", "--deriver", str(output)],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=30, check=False)
            require(queried.returncode == 0 and queried.stdout.strip() == row["derivation"],
                    "nix_store_binding", f"{name}: Nix store deriver mismatch: {queried.stdout.strip()} {queried.stderr.strip()}")
            verified = subprocess.run(["nix-store", "--verify-path", str(output)],
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=60, check=False)
            require(verified.returncode == 0, "nix_store_verify", f"{name}: Nix store path did not verify: {verified.stderr.strip()}")
            record["store_verified"] = True
        records[name] = record
    require(set(records) == expected_names, "nix_provenance", f"missing Nix components: {sorted(expected_names - set(records))}")
    return {"path": str(path), "sha256": sha256_file(path), "records": records}


@dataclasses.dataclass
class ProcessIdentity:
    pid: int
    start_time_ticks: int
    executable: str

    def as_dict(self) -> dict:
        return dataclasses.asdict(self)


def process_identity(pid: int) -> ProcessIdentity | None:
    try:
        raw = Path(f"/proc/{pid}/stat").read_text()
        closing = raw.rfind(")")
        fields = raw[closing + 2:].split()
        start_ticks = int(fields[19])
        executable = os.readlink(f"/proc/{pid}/exe")
    except (FileNotFoundError, ProcessLookupError, PermissionError, ValueError, OSError):
        return None
    return ProcessIdentity(pid, start_ticks, executable)


def process_has_token(pid: int, token: str) -> bool:
    try:
        entries = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
    except (FileNotFoundError, ProcessLookupError, PermissionError, OSError):
        return False
    expected = f"{TOKEN_VARIABLE}={token}".encode()
    return expected in entries


def token_survivors(token: str) -> list[ProcessIdentity]:
    survivors = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        pid = int(entry.name)
        if process_has_token(pid, token):
            identity = process_identity(pid)
            if identity is not None:
                survivors.append(identity)
    return survivors


class RunDirectory:
    def __init__(self, value: Path):
        require(value.is_absolute(), "run_dir", "run directory must be absolute")
        valid_prefix = any(value.parent == prefix.parent and value.name.startswith(prefix.name) and
                           re.fullmatch(re.escape(prefix.name) + r"[A-Za-z0-9][A-Za-z0-9_.-]{7,100}", value.name) is not None
                           for prefix in RUN_PREFIXES)
        require(valid_prefix, "run_dir", "run directory must be a unique /tmp/opencode/LP-v09-live-* or LP-v09-r2-* path")
        require(not value.is_symlink(), "run_dir", "run directory must not be a symlink")
        if value.exists():
            require(value.is_dir() and not any(value.iterdir()), "run_dir_not_empty", f"run directory is not new/empty: {value}")
        self.root = value
        self.private = value / "private"
        self.public = value / "public"
        self.token = secrets.token_hex(32)
        for directory in [value, self.private, self.public]:
            directory.mkdir(parents=False, exist_ok=True, mode=0o700)
            os.chmod(directory, 0o700)


class OwnedProcess:
    def __init__(self, name: str, argv: list[str], cwd: Path, log_path: Path, token: str,
                 extra_env: dict[str, str] | None = None):
        log_path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.log_path = log_path
        self.log_handle = log_path.open("xb", buffering=0)
        os.chmod(log_path, 0o600)
        environment = os.environ.copy()
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        environment[TOKEN_VARIABLE] = token
        environment.update(extra_env or {})
        self.popen = subprocess.Popen(
            argv, cwd=cwd, stdin=subprocess.DEVNULL, stdout=self.log_handle,
            stderr=subprocess.STDOUT, env=environment, start_new_session=True,
            preexec_fn=lambda: os.umask(0o077),
        )
        self.name = name
        self.argv = argv
        self.token = token
        self.identity = process_identity(self.popen.pid)
        require(self.identity is not None, "process_identity", f"could not capture {name} process identity")

    def owned(self) -> bool:
        current = process_identity(self.popen.pid)
        # A supervisor such as nix develop may exec Python in the same PID.
        # The kernel start token and our cryptographic ownership token remain;
        # executable-path text alone must not turn that same process foreign.
        return (current is not None and current.pid == self.identity.pid and
                current.start_time_ticks == self.identity.start_time_ticks and
                process_has_token(self.popen.pid, self.token))

    def signal(self, sig: signal.Signals) -> None:
        if self.popen.poll() is None:
            require(self.owned(), "process_ownership_refused", f"refusing to signal replaced/non-owned {self.name}")
            self.popen.send_signal(sig)

    def close_log(self) -> None:
        if not self.log_handle.closed:
            self.log_handle.close()


def stop_owned_token(token: str, grace_seconds: float = 10.0) -> list[dict]:
    actions = []
    for sig, wait in [(signal.SIGTERM, grace_seconds), (signal.SIGKILL, 3.0)]:
        for identity in token_survivors(token):
            if process_has_token(identity.pid, token) and process_identity(identity.pid) == identity:
                try:
                    os.kill(identity.pid, sig)
                    actions.append({"pid": identity.pid, "signal": sig.name, "start_time_ticks": identity.start_time_ticks})
                except ProcessLookupError:
                    pass
        deadline = time.monotonic() + wait
        while time.monotonic() < deadline and token_survivors(token):
            time.sleep(0.1)
    require(not token_survivors(token), "process_cleanup", "owned-token processes survived bounded SIGTERM/SIGKILL cleanup")
    return actions


def secure_tree(root: Path) -> dict:
    """Make only this newly owned artifact tree private; never follow symlinks."""
    changed = []
    for directory, dirnames, filenames in os.walk(root, topdown=True, followlinks=False):
        base = Path(directory)
        for name in list(dirnames):
            path = base / name
            require(not path.is_symlink(), "artifact_symlink", f"artifact symlink refused: {path}")
            if stat.S_IMODE(path.stat().st_mode) != 0o700:
                changed.append({"path": str(path), "from": oct(stat.S_IMODE(path.stat().st_mode)), "to": "0o700"})
                os.chmod(path, 0o700)
        for name in filenames:
            path = base / name
            info = path.lstat()
            require(not stat.S_ISLNK(info.st_mode), "artifact_symlink", f"artifact symlink refused: {path}")
            require(stat.S_ISREG(info.st_mode) or stat.S_ISFIFO(info.st_mode) or stat.S_ISSOCK(info.st_mode),
                    "artifact_type", f"unexpected artifact type: {path}")
            if stat.S_IMODE(info.st_mode) != 0o600:
                changed.append({"path": str(path), "from": oct(stat.S_IMODE(info.st_mode)), "to": "0o600"})
                os.chmod(path, 0o600, follow_symlinks=False)
    if stat.S_IMODE(root.stat().st_mode) != 0o700:
        os.chmod(root, 0o700)
        changed.append({"path": str(root), "to": "0o700"})
    return {"private_files": "0600", "private_directories": "0700", "mode_changes": changed,
            "file_count": len(changed)}
