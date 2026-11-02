# Local parity with CI. Everything that ships goes through Nix.
set shell := ["bash", "-euo", "pipefail", "-c"]

default: check

check: fmt clippy test

fmt:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

e2e:
    ./testing/regtest-e2e.sh

ark-e2e:
    SPC_ARK_E2E=1 ./testing/ark-e2e.sh
