mod common;

use spc_lab::manifest::{LockError, RunLock};
use spc_lab::RunId;

#[test]
fn kernel_lock_rejects_a_second_live_owner_and_is_reusable_after_drop() {
    let dir = common::TestDir::new("run-lock");
    let path = dir.join("owner.lock");
    let run = RunId::new("run-lock").unwrap();
    let first = RunLock::acquire(&path, &run, "owner-a").unwrap();
    let error = RunLock::acquire(&path, &run, "owner-b").unwrap_err();
    assert!(matches!(error, LockError::AlreadyLocked { .. }));
    drop(first);
    let third = RunLock::acquire(&path, &run, "owner-c").unwrap();
    drop(third);
}
