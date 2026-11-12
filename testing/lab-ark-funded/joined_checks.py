"""Feature-aware joined admission. Source lineage is not compiled coverage."""
from pathlib import Path

from common import artifact, require, verify_source

UI_FILES = {"bins/spc-lab/src/main.rs", "crates/spc-lab/src/http.rs",
            "bins/spc-lab/static/index.html", "bins/spc-lab/static/app.js", "bins/spc-lab/static/style.css",
            "bins/spc-lab/static/lab.html", "bins/spc-lab/static/lab.js", "bins/spc-lab/static/lab.css"}


def cargo_command(argv, verb, package, ark=False):
    require(type(argv) is list and argv and all(type(v) is str for v in argv), "joined check needs actual argv")
    require(Path(argv[0]).name == "cargo" and argv[1:2] == [verb], "joined check is not the claimed cargo invocation")
    args = argv[2:]
    packages = [args[i+1] for i, v in enumerate(args[:-1]) if v in ("-p", "--package")]
    packages += [v.split("=", 1)[1] for v in args if v.startswith("--package=")]
    require(package in packages, "joined check omits required package")
    require(not any(v in args for v in ("--no-run", "--list", "--ignored", "--exclude")), "joined check did not execute required tests")
    if ark:
        features = [args[i+1] for i, v in enumerate(args[:-1]) if v == "--features"]
        features += [v.split("=", 1)[1] for v in args if v.startswith("--features=")]
        tokens = {f for value in features for f in value.replace(",", " ").split()}
        require("ark" in tokens or package + "/ark" in tokens, "explicit Ark-feature invocation required; default agent tests are not wallet coverage")


def validate_joined_features(validation, bundle, source_root, manifest):
    checks = validation["checks"]
    required = {"agent-ark-tests", "agent-ark-build", "pool-ark-tests", "pool-ark-build", "ui-build", "ui-browser-tests"}
    selected = {c.get("purpose"): c for c in checks if c.get("purpose") in required}
    require(set(selected) == required, "joined receipt lacks explicit Ark-feature and complete compiled UI gates")
    require(sum(c.get("purpose") in required for c in checks) == len(required), "duplicate joined feature gate")
    for purpose, check in selected.items():
        require(type(check.get("exit")) is int and check["exit"] == 0
                and check.get("source_manifest_sha256") == bundle["joined_source_manifest"]["sha256"], "joined check failed or covers different source")
        log = artifact(check["log"]).read_text()
        if purpose.startswith(("agent-", "pool-")):
            name = purpose.split("-", 1)[0]
            verb = "test" if purpose.endswith("tests") else "build"
            cargo_command(check.get("argv"), verb, "spc-" + name, ark=True)
            if verb == "test":
                require("test result: ok." in log and "0 failed" in log, "Ark test log lacks executed passing tests")
                marker = "wallet::" if name == "agent" else "ark::"
                require(any(line.startswith("test ") and marker in line and line.rstrip().endswith(" ... ok") for line in log.splitlines()), "Ark test log has no actual wallet/Ark test execution")
            else:
                require(check.get("binary_sha256") == bundle["binaries"][name]["sha256"], "Ark build does not bind admitted binary")
        elif purpose == "ui-build":
            cargo_command(check.get("argv"), "build", "spc-lab-cli")
            require(check.get("binary_sha256") == bundle["binaries"]["lab"]["sha256"], "UI build does not bind admitted CLI")
        else:
            require(check.get("argv") and check.get("tested_binary_sha256") == bundle["binaries"]["lab"]["sha256"], "browser tests did not exercise admitted compiled UI")
            require(check.get("required_cases") and check.get("passed_cases") == check["required_cases"], "complete compiled browser case census required")
    files = validation.get("ui_source_files", {})
    require(UI_FILES <= set(files), "joined UI source census omits base/http/DOM assets")
    verify_source(source_root, manifest, files)
