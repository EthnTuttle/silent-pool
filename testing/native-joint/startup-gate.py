"""Focused, same-source compile/execute gate; parent owns build slot/disk guard.

Use inside the existing same-source Nix joint check hooks. Cargo supplies the
agent harness artifact; no runtime-supplied expected hash/path is accepted.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess


def run(command, env):
    subprocess.run(command, env=env, check=True)


def main():
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    # These are the same build-generated anchors as the portable CPU check.
    for key in ("SPC_BUILD_SOURCE_ID", "SPC_JOINT_TEST_SOURCE", "SPC_JOINT_DESCRIPTOR_SHA256"):
        if not env.get(key):
            raise RuntimeError("missing same-source Nix build anchor: " + key)
    agent = ["cargo", "test", "--offline", "--locked", "-p", "spc-agent", "--features", "ark", "--lib"]
    output = subprocess.run(agent + ["--no-run", "--message-format=json"], env=env,
                            check=True, stdout=subprocess.PIPE, text=True)
    artifacts = []
    for line in output.stdout.splitlines():
        value = json.loads(line)
        if (value.get("reason") == "compiler-artifact"
                and value.get("target", {}).get("name") == "spc_agent"
                and value.get("profile", {}).get("test") is True
                and value.get("executable")):
            artifacts.append(Path(value["executable"]).resolve(strict=True))
    if len(artifacts) != 1:
        raise RuntimeError("ambiguous/missing compiled agent harness")
    digest = hashlib.sha256()
    with artifacts[0].open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    env["SPC_NATIVE_STARTUP_AGENT_HARNESS"] = str(artifacts[0])
    env["SPC_NATIVE_STARTUP_AGENT_SHA256"] = digest.hexdigest()
    run(agent + ["wallet::native_audit::tests::startup_tests::"], env)
    lab = ["cargo", "test", "--offline", "--locked", "-p", "spc-lab", "--lib"]
    prefix = "wallet_bridge::native_render::run_token_tests::"
    run(lab + ["wallet_bridge::native_render::"], env)
    run(lab + [prefix + "native_run_token_gate", "--", "--ignored", "--exact", "--nocapture"], env)
    # Print hashes only. Raw tokens, private configs and process env stay private.
    print(json.dumps({"schema": "spc.native-startup-gate/1", "source_id": env["SPC_BUILD_SOURCE_ID"],
                      "source_path": env["SPC_JOINT_TEST_SOURCE"],
                      "joint_descriptor_sha256": env["SPC_JOINT_DESCRIPTOR_SHA256"],
                      "agent_harness_sha256": digest.hexdigest(),
                      "scope": "real engine token generation/render, Supervisor export, loaded agent config and native service publication; fixture infrastructure; no funded/full-controller acceptance"}))


if __name__ == "__main__":
    main()
