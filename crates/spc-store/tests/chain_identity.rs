//! Real persistence test. Requires explicitly selected disposable Postgres.
use spc_crypto::ChainNetwork;
use spc_store::{chain::ChainBinding, Store};

#[tokio::test]
#[ignore = "requires disposable SPC_TEST_DATABASE_URL and CREATE DATABASE privilege"]
async fn reopened_db_rejects_other_test_chain_without_rebinding() {
    let url = std::env::var("SPC_TEST_DATABASE_URL").expect("disposable DB URL required");
    let base = url.split('?').next().unwrap();
    let (head, _) = base.rsplit_once('/').unwrap();
    let name = format!("spc_chain_{}", uuid::Uuid::new_v4().simple());
    let admin = sqlx::PgPool::connect(&format!("{head}/postgres"))
        .await
        .unwrap();
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let target = format!("{head}/{name}");
    let store = Store::connect(&target).await.unwrap();
    store.preflight_chain(ChainNetwork::Testnet4).await.unwrap();
    store.migrate().await.unwrap();
    store
        .initialize_chain(ChainNetwork::Testnet4)
        .await
        .unwrap();
    let before = store.require_chain(ChainNetwork::Testnet4).await.unwrap();
    store.pool().close().await;
    let reopened = Store::connect(&target).await.unwrap();
    assert!(reopened
        .preflight_chain(ChainNetwork::Testnet5)
        .await
        .is_err());
    assert!(reopened
        .require_chain(ChainNetwork::Testnet5)
        .await
        .is_err());
    assert!(reopened
        .initialize_chain(ChainNetwork::Testnet5)
        .await
        .is_err());
    assert_eq!(
        reopened
            .require_chain(ChainNetwork::Testnet4)
            .await
            .unwrap(),
        before
    );
    assert_eq!(before, ChainBinding::new(ChainNetwork::Testnet4));
    reopened.pool().close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires exclusively allocated EMPTY SPC_TEST_REDIS_URL database"]
async fn redis_namespace_rejects_nonempty_foreign_and_damaged_binding() {
    use redis::AsyncCommands;
    use spc_store::chain::bind_redis_namespace;
    let url = std::env::var("SPC_TEST_REDIS_URL").expect("exclusive empty test Redis DB required");
    let client = redis::Client::open(url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let size: i64 = redis::cmd("DBSIZE").query_async(&mut conn).await.unwrap();
    assert_eq!(size, 0, "test refuses to touch a nonempty Redis database");
    let marker = format!("spc:chain-test:{}", uuid::Uuid::new_v4());
    let _: () = conn.set(&marker, "owned-test-marker").await.unwrap();
    assert!(bind_redis_namespace(&url, ChainNetwork::Testnet5, true)
        .await
        .is_err());
    let preserved: String = conn.get(&marker).await.unwrap();
    assert_eq!(preserved, "owned-test-marker");
    let _: i64 = conn.del(&marker).await.unwrap();
    assert!(bind_redis_namespace(&url, ChainNetwork::Testnet5, false)
        .await
        .is_err());
    bind_redis_namespace(&url, ChainNetwork::Testnet5, true)
        .await
        .unwrap();
    bind_redis_namespace(&url, ChainNetwork::Testnet5, false)
        .await
        .unwrap();
    let before: String = conn.get("spc:chain-identity").await.unwrap();
    assert!(bind_redis_namespace(&url, ChainNetwork::Testnet4, true)
        .await
        .is_err());
    assert!(bind_redis_namespace(&url, ChainNetwork::Testnet4, false)
        .await
        .is_err());
    let after: String = conn.get("spc:chain-identity").await.unwrap();
    assert_eq!(before, after);
    let _: () = conn.set("spc:chain-identity", "{}").await.unwrap();
    assert!(bind_redis_namespace(&url, ChainNetwork::Testnet5, true)
        .await
        .is_err());
    let damaged: String = conn.get("spc:chain-identity").await.unwrap();
    assert_eq!(damaged, "{}");
    // Only the two exact keys this test created are ever deleted, no FLUSHDB.
    let _: i64 = conn.del("spc:chain-identity").await.unwrap();
}
