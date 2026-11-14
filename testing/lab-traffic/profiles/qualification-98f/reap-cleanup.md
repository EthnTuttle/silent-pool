    Updating crates.io index
warning: failed to write cache, path: /home/ethan/.cargo/registry/index/index.crates.io-1949cf8c6b5b557f/.cache/ar/_a/ar_archive_writer, error: Permission denied (os error 13)
warning: failed to auto-clean cache data

failed to clean entries from the global cache

Caused by:
  failed to remove file `/home/ethan/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wayland-scanner-0.29.5/README.md`

Caused by:
  Permission denied (os error 13)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 4.41s
warning: the following packages contain code that will be rejected by a future version of Rust: sqlx-postgres v0.8.0
note: to see what the problems were, use the option `--future-incompat-report`, or run `cargo report future-incompatibilities --id 1`
     Running unittests src/lib.rs (target/debug/deps/spc_lab-f3bcfb8750da4fc3)

running 12 tests
test engine::traffic_runtime::reap_cleanup_tests::persisted_bindings_reject_changed_argv ... ok
test engine::traffic_runtime::reap_cleanup_tests::never_launched_nodes_are_not_a_cleanup_failure ... ok
test engine::traffic_runtime::reap_cleanup_tests::persisted_bindings_reject_changed_binary ... ok
test engine::traffic_runtime::reap_cleanup_tests::cleanup_rejects_a_present_but_invalid_capability_record ... ok
test engine::traffic_runtime::reap_cleanup_tests::persisted_bindings_accept_a_stopped_run ... ok
test engine::traffic_runtime::reap_cleanup_tests::live_reverify_refuses_what_cleanup_validation_accepts ... ok
test engine::traffic_runtime::reap_cleanup_tests::missing_process_record_is_not_a_cleanup_failure ... ok
test engine::traffic_runtime::reap_cleanup_tests::persisted_bindings_reject_record_without_profile ... ok
test engine::traffic_runtime::reap_cleanup_tests::persisted_bindings_reject_changed_driver_config ... ok
test engine::traffic_runtime::reap_cleanup_tests::partial_provisioning_reaps_the_never_launched_node_too ... ok
test engine::traffic_runtime::reap_cleanup_tests::reap_never_signals_a_process_it_does_not_own ... ok
test engine::traffic_runtime::reap_cleanup_tests::cleanup_attaches_a_never_granted_capability_run ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 106 filtered out; finished in 0.17s

