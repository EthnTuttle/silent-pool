    Updating crates.io index
warning: failed to write cache, path: /home/ethan/.cargo/registry/index/index.crates.io-1949cf8c6b5b557f/.cache/ar/_a/ar_archive_writer, error: Permission denied (os error 13)
warning: failed to auto-clean cache data

failed to clean entries from the global cache

Caused by:
  failed to remove file `/home/ethan/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wayland-scanner-0.29.5/README.md`

Caused by:
  Permission denied (os error 13)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 7.03s
warning: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
note: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
     Running unittests src/lib.rs (target/debug/deps/spc_agent-f760ac42d8402846)

running 1 test
test wallet::native_audit::tests::startup_tests::foreign_network_namespace_is_rejected_before_token_authorization ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 184 filtered out; finished in 0.03s

