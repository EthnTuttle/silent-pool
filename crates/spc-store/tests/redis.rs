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

// --- Reader/publisher connection split regressions (2026-11-06) -----------

use std::time::{Duration, Instant};

use futures_util::StreamExt;

fn redis_url() -> Option<String> {
    std::env::var("SPC_TEST_REDIS_URL").ok()
}

async fn raw_client(url: &str) -> redis::Client {
    redis::Client::open(url).expect("redis url")
}

type MonitorStream = std::pin::Pin<Box<dyn futures_util::Stream<Item = String> + Send>>;

/// Start MONITOR before the reader so the blocking command cannot be missed.
async fn monitor_stream(url: &str) -> MonitorStream {
    let client = raw_client(url).await;
    let mut monitor = client.get_async_monitor().await.expect("monitor connect");
    monitor.monitor().await.expect("MONITOR");
    Box::pin(monitor.into_on_message::<String>())
}

/// Wait until MONITOR reports the exact blocking read for `stream`.
async fn wait_for_xreadgroup(mut messages: MonitorStream, stream: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("reader did not enter XREADGROUP for {stream}");
        }
        match tokio::time::timeout(remaining, messages.next()).await {
            Ok(Some(message)) if message.contains("XREADGROUP") && message.contains(stream) => {
                return;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("MONITOR stream ended"),
            Err(_) => panic!("reader did not enter XREADGROUP for {stream}"),
        }
    }
}

#[tokio::test]
async fn isolated_reader_connection_does_not_block_publisher_xadd() {
    let Some(url) = redis_url() else {
        return;
    };
    let stream = format!("spc:test:reader-split:{}", uuid::Uuid::new_v4());
    let group = "grp";
    let reader = Streams::connect(&url).await.expect("reader connect");
    let publisher = Streams::connect(&url).await.expect("publisher connect");
    reader
        .ensure_group(&stream, group)
        .await
        .expect("ensure group");

    let monitor = monitor_stream(&url).await;
    let reader_task = {
        let reader = reader.clone();
        let stream = stream.clone();
        tokio::spawn(async move {
            reader
                .read_group(&stream, group, "consumer", 16)
                .await
                .expect("blocking read")
        })
    };
    wait_for_xreadgroup(monitor, &stream).await;

    let payload = spc_store::wire::FrontEvent::TipHeartbeat {
        rev: 1,
        tip_hash: [0x42; 32],
        height: 1,
        unix_ms: 1,
    }
    .encode()
    .expect("encode");
    let started = Instant::now();
    publisher
        .add(&stream, &payload)
        .await
        .expect("publish must not wait for the reader timeout");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(250),
        "isolated XADD took {elapsed:?}; the reader still shares the publisher client"
    );

    let entries = tokio::time::timeout(Duration::from_secs(2), reader_task)
        .await
        .expect("reader must receive the published entry")
        .expect("reader task");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].1, payload);

    let pending = reader
        .read_pending(&stream, group, "consumer", 16)
        .await
        .expect("read pending");
    assert_eq!(pending.len(), 1);
    reader
        .ack(&stream, group, &pending[0].0)
        .await
        .expect("ack");
    let pending = reader
        .read_pending(&stream, group, "consumer", 16)
        .await
        .expect("read pending after ack");
    assert!(pending.is_empty());

    let client = raw_client(&url).await;
    let mut cleanup = client
        .get_multiplexed_async_connection()
        .await
        .expect("cleanup connect");
    let _: i64 = redis::cmd("DEL")
        .arg(&stream)
        .query_async(&mut cleanup)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn one_raw_client_still_reproduces_blocking_xadd_serialization() {
    let Some(url) = redis_url() else {
        return;
    };
    let stream = format!("spc:test:reader-shared:{}", uuid::Uuid::new_v4());
    let group = "grp";
    let name = format!("shared-{}", uuid::Uuid::new_v4());
    let client = raw_client(&url).await;
    let mut shared = client
        .get_multiplexed_async_connection()
        .await
        .expect("connect");
    let mut observer = client
        .get_multiplexed_async_connection()
        .await
        .expect("observer connect");
    let _: () = redis::cmd("CLIENT")
        .arg("SETNAME")
        .arg(&name)
        .query_async(&mut shared)
        .await
        .expect("setname");
    let _: () = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(&stream)
        .arg(group)
        .arg("0")
        .arg("MKSTREAM")
        .query_async(&mut shared)
        .await
        .expect("group");

    let reader_task = {
        let mut reader = shared.clone();
        let stream = stream.clone();
        tokio::spawn(async move {
            let value: redis::Value = redis::cmd("XREADGROUP")
                .arg("GROUP")
                .arg(group)
                .arg("consumer")
                .arg("COUNT")
                .arg(16)
                .arg("BLOCK")
                .arg(1000)
                .arg("STREAMS")
                .arg(&stream)
                .arg(">")
                .query_async(&mut reader)
                .await
                .expect("blocking raw read");
            value
        })
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let list: String = redis::cmd("CLIENT")
            .arg("LIST")
            .query_async(&mut observer)
            .await
            .expect("client list");
        if list
            .lines()
            .any(|line| line.contains(&format!("name={name}")) && line.contains("flags=b"))
        {
            break;
        }
        assert!(Instant::now() < deadline, "reader did not block");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let started = Instant::now();
    let _: String = redis::cmd("XADD")
        .arg(&stream)
        .arg("*")
        .arg("payload")
        .arg("x")
        .query_async(&mut shared)
        .await
        .expect("same-client XADD");
    let elapsed = started.elapsed();
    assert!(
        elapsed > Duration::from_millis(400),
        "shared-client XADD returned in {elapsed:?}; the negative topology no longer reproduces"
    );
    let _ = tokio::time::timeout(Duration::from_secs(2), reader_task)
        .await
        .expect("raw reader must complete")
        .expect("raw reader task");

    let _: i64 = redis::cmd("DEL")
        .arg(&stream)
        .query_async(&mut shared)
        .await
        .expect("cleanup");
}

/// `kv_set_ex` writes atomically with a TTL, `scan_keys` finds exactly the
/// matching keys, and expiry removes a snapshot from the scan (the
/// `spc:metrics:*` staleness contract).
#[tokio::test]
async fn kv_set_ex_and_scan_keys_round_trip() {
    let Some(streams) = streams().await else {
        return;
    };
    let ns = format!("spc:test:metrics:{}", uuid::Uuid::new_v4());
    let a = format!("{ns}:front:0");
    let b = format!("{ns}:front:1");
    streams.kv_set_ex(&a, "{\"x\":1}", 60).await.unwrap();
    streams.kv_set_ex(&b, "{\"x\":2}", 60).await.unwrap();
    streams
        .kv_set_ex(&format!("{ns}:ark"), "{}", 60)
        .await
        .unwrap();

    let keys = streams.scan_keys(&format!("{ns}:front:*")).await.unwrap();
    assert_eq!(keys, vec![a.clone(), b.clone()]);
    assert_eq!(
        streams.kv_get(&a).await.unwrap().as_deref(),
        Some("{\"x\":1}")
    );

    // TTL 1 s: the key must be gone from both GET and SCAN after expiry.
    streams.kv_set_ex(&a, "{\"x\":1}", 1).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(streams.kv_get(&a).await.unwrap(), None);
    let keys = streams.scan_keys(&format!("{ns}:front:*")).await.unwrap();
    assert_eq!(keys, vec![b]);
}

/// `group_lag_ms`: absent stream and group-less stream are unknown (`None`),
/// a caught-up group reports 0, and an undelivered entry reports a positive
/// lag bounded by the entry's age.
#[tokio::test]
async fn group_lag_measures_delivery_lag() {
    let Some(streams) = streams().await else {
        return;
    };
    let stream = format!("spc:test:lag:{}", uuid::Uuid::new_v4());
    // Never written: unknown.
    assert_eq!(streams.group_lag_ms(&stream).await.unwrap(), None);

    streams.add(&stream, b"one").await.unwrap();
    // No consumer group yet: still unknown, not zero.
    assert_eq!(streams.group_lag_ms(&stream).await.unwrap(), None);

    streams.ensure_group_from(&stream, "g", "0").await.unwrap();
    // Group exists but has not read the entry: positive lag, at most the
    // entry's age (which is below 60 s in any healthy test run).
    let lag = streams.group_lag_ms(&stream).await.unwrap().unwrap();
    assert!(lag > 0, "undelivered entry must show lag");
    assert!(lag < 60_000, "lag {lag} ms exceeds the test's own runtime");

    let entries = streams.read_group(&stream, "g", "c", 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    // Delivered (even unacked): caught up.
    assert_eq!(streams.group_lag_ms(&stream).await.unwrap(), Some(0));
}
