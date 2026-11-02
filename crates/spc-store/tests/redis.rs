//! Redis helper integration tests. Skipped unless `SPC_TEST_REDIS_URL`
//! points at a throwaway server (the e2e harness sets it).

use spc_store::redis::Streams;

async fn streams() -> Option<Streams> {
    let url = std::env::var("SPC_TEST_REDIS_URL").ok()?;
    Some(Streams::connect(&url).await.expect("connect"))
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
