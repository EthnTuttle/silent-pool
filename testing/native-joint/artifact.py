"""Build-only descriptor generator. No runtime hash overrides or daemon query.

Inputs are Nix derivation arguments, never wallet fixture metadata. The caller
must be a trusted same-source Nix check (see README.md).
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import stat
import subprocess


def require(condition, message):
    if not condition:
        raise ValueError(message)


def store_path(value):
    require(re.fullmatch(r"/nix/store/[0-9abcdfghijklmnpqrsvwxyz]{32}-[A-Za-z0-9+._?=-]+", value),
            "expected a direct Nix store path")
    return Path(value)


def sha_file(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


class ATerm:
    """Bounded parser for classic input-addressed Derive tuples; reject drift."""
    def __init__(self, text):
        require(len(text) <= 16 * 1024 * 1024, "derivation size")
        self.text, self.pos = text, 0

    def take(self, value):
        require(self.text.startswith(value, self.pos), "derivation syntax")
        self.pos += len(value)

    def value(self, depth=0):
        require(depth < 16 and self.pos < len(self.text), "derivation nesting")
        char = self.text[self.pos]
        if char == '"':
            self.pos += 1
            result = []
            while self.pos < len(self.text):
                char = self.text[self.pos]
                self.pos += 1
                if char == '"':
                    return "".join(result)
                if char == "\\":
                    require(self.pos < len(self.text), "truncated escape")
                    char = self.text[self.pos]
                    self.pos += 1
                    require(char in '\\"nrt', "unsupported derivation escape")
                    char = {"n": "\n", "r": "\r", "t": "\t"}.get(char, char)
                result.append(char)
            raise ValueError("unterminated string")
        require(char in "[(", "expected derivation collection")
        end = "]" if char == "[" else ")"
        self.pos += 1
        result = []
        if not self.text.startswith(end, self.pos):
            while True:
                result.append(self.value(depth + 1))
                if not self.text.startswith(",", self.pos):
                    break
                self.pos += 1
        self.take(end)
        return result

    def parse(self):
        self.take("Derive")
        result = self.value()
        require(self.pos == len(self.text) and len(result) == 7, "unsupported derivation")
        return result


def correlate(raw, output, source, source_id):
    outputs, _, input_sources, _, _, _, env_rows = ATerm(raw).parse()
    require(all(isinstance(row, list) and len(row) == 2
                and all(isinstance(x, str) for x in row) for row in env_rows), "env shape")
    env = dict(env_rows)
    require(len(env) == len(env_rows), "duplicate env")
    require(outputs == [["out", output, "", ""]], "CPU output/deriver mismatch")
    require(env.get("out") == output, "CPU env output mismatch")
    require(env.get("src") == source and source in input_sources, "CPU source mismatch")
    require(env.get("SPC_BUILD_SOURCE_ID") == source_id, "CPU source identity mismatch")
    require(source_id == "nix-snapshot" or re.fullmatch(r"git:[0-9a-f]{40}", source_id),
            "invalid source identity")


def generate(args):
    output, source, drv = map(store_path, (args.cpu_output, args.source, args.cpu_drv))
    require(str(drv).endswith(".drv"), "derivation suffix")
    for path in (output, source, drv):
        require(path.resolve(strict=True) == path, "store input alias")
        require(not path.stat().st_mode & 0o222, "mutable store input")
    require(drv.stat().st_size <= 16 * 1024 * 1024, "derivation bound")
    raw = drv.read_bytes()
    correlate(raw.decode(), str(output), str(source), args.source_id)
    binary = output / "bin/spc-cpu-miner"
    require(binary.resolve(strict=True) == binary, "CPU path alias")
    meta = binary.lstat()
    require(stat.S_ISREG(meta.st_mode) and meta.st_mode & 0o111
            and not meta.st_mode & 0o222, "CPU must be an immutable regular executable")
    require(0 < meta.st_size <= 1024 * 1024 * 1024, "CPU size bound")
    # --dump serializes the actual source filesystem, without store DB/daemon.
    nar_hash = hashlib.sha256()
    with subprocess.Popen([args.nix_store, "--dump", str(source)], stdout=subprocess.PIPE) as proc:
        for chunk in iter(lambda: proc.stdout.read(1024 * 1024), b""):
            nar_hash.update(chunk)
        require(proc.wait() == 0, "source NAR dump failed")
    descriptor = {
        "schema": "spc.native-joint-artifact/1",
        "kind": "same-source-nix",
        "binary_path": str(binary), "binary_sha256": sha_file(binary),
        "binary_bytes": meta.st_size, "source_id": args.source_id,
        "source_path": str(source), "source_nar_sha256": nar_hash.hexdigest(),
        "derivation_path": str(drv),
        "derivation_sha256": hashlib.sha256(raw).hexdigest(), "output_path": str(output),
    }
    destination = Path(args.destination)
    destination.mkdir()
    data = (json.dumps(descriptor, sort_keys=True, separators=(",", ":")) + "\n").encode()
    require(len(data) <= 8192, "descriptor size bound")
    (destination / "descriptor.json").write_bytes(data)
    anchors = {
        "SPC_JOINT_DESCRIPTOR": str(destination / "descriptor.json"),
        "SPC_JOINT_DESCRIPTOR_SHA256": hashlib.sha256(data).hexdigest(),
        "SPC_JOINT_SOURCE_NAR_SHA256": descriptor["source_nar_sha256"],
        "SPC_JOINT_CPU_DRV": str(drv),
        "SPC_JOINT_CPU_DRV_SHA256": descriptor["derivation_sha256"],
        "SPC_JOINT_CPU_OUTPUT": str(output),
    }
    (destination / "anchors.sh").write_text("".join(
        f"export {key}={shlex.quote(value)}\n" for key, value in anchors.items()))
    for path in destination.iterdir():
        path.chmod(0o444)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    for name in ("cpu-output", "cpu-drv", "source", "source-id", "destination", "nix-store"):
        parser.add_argument("--" + name, required=True)
    generate(parser.parse_args())
