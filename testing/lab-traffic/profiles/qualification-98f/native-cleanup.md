    Updating crates.io index
warning: failed to write cache, path: /home/ethan/.cargo/registry/index/index.crates.io-1949cf8c6b5b557f/.cache/ar/_a/ar_archive_writer, error: Permission denied (os error 13)
warning: failed to auto-clean cache data

failed to clean entries from the global cache

Caused by:
  failed to remove file `/home/ethan/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wayland-scanner-0.29.5/README.md`

Caused by:
  Permission denied (os error 13)
   Compiling spc-lab v0.1.0 (/home/ethan/code/silent-pool/crates/spc-lab)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 22.72s
warning: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
note: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
     Running unittests src/lib.rs (target/debug/deps/spc_lab-f3bcfb8750da4fc3)

running 8 tests
test engine::native_audit::native_audit_cleanup_tests::cleanup_rejects_a_foreign_native_lease_projection ... ok
test engine::native_audit::native_audit_cleanup_tests::a_present_lease_without_a_wallet_record_rejects ... ok
test engine::native_audit::native_audit_cleanup_tests::a_present_lease_with_a_rewritten_registry_rejects ... ok
test engine::native_audit::native_audit_cleanup_tests::a_present_lease_with_a_replaced_wallet_record_rejects ... ok
test engine::native_audit::native_audit_cleanup_tests::cleanup_never_signals_a_foreign_wallet_pid ... ok
test engine::native_audit::native_audit_cleanup_tests::a_present_lease_with_a_replaced_miner_peer_record_rejects ... ok
test engine::native_audit::native_audit_cleanup_tests::a_present_lease_projection_is_validated_as_persisted_metadata ... ok
test engine::native_audit::native_audit_cleanup_tests::cleanup_attaches_a_partial_native_wallet_startup_run ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 110 filtered out; finished in 0.18s

