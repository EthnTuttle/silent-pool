//! Redis helper integration tests. Skipped unless `SPC_TEST_REDIS_URL`
//! points at a throwaway server (the e2e harness sets it).

use spc_store::redis::Streams;

async fn streams() -> Option<Streams> {
    let url = std::env::var("SPC_TEST_REDIS_URL").ok()?;
    Some(Streams::connect(&url).await.expect("connect"))
}

/// A group created with an explicit start id delivers entries that existed
/// before the group (the `spc:ark:receipts` restart path); re-creating it is
/// a no-op, not a reset.
#[tokio::test]
async fn group_from_zero_delivers_preexisting_entries() {
    let Some(streams) = streams().await else {
        return;
    };
    let stream = format!("spc:test:grp:{}", uuid::Uuid::new_v4());
    streams.add(&stream, b"first").await.unwrap();
    let group = "g";
    streams
        .ensure_group_from(&stream, group, "0")
        .await
        .unwrap();

    let entries = streams.read_group(&stream, group, "c", 10).await.unwrap();
    assert!(entries
        .iter()
        .any(|(_, payload)| payload.as_slice() == b"first"));

    // BUSYGROUP: the existing group (and its cursor) is kept.
    streams
        .ensure_group_from(&stream, group, "0")
        .await
        .unwrap();
}

/// The Ark order-dedupe helpers (review 7.6): SADD is the atomic
/// mark-if-absent, SISMEMBER sees the mark, and EXPIRE bounds retention to
/// the retry window once orders stop.
#[tokio::test]
async fn set_helpers_mark_and_expire() {
    let Some(streams) = streams().await else {
        return;
    };
    let key = format!("spc:test:seen:{}", uuid::Uuid::new_v4());
    let member = [7u8; 16];

    assert!(streams.sadd(&key, &member).await.unwrap());
    assert!(!streams.sadd(&key, &member).await.unwrap());
    assert!(streams.sismember(&key, &member).await.unwrap());
    assert!(!streams.sismember(&key, &[8u8; 16]).await.unwrap());

    // TTL = 1 s doubles as the test's cleanup.
    streams.expire(&key, 1).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    assert!(!streams.sismember(&key, &member).await.unwrap());
}
