//! SPc observation ingestion. Spawned by payout, the sole Postgres writer.
use spc_store::{
    observations::{
        observation_digest, FrontObservation, FrontObservationAck, FRONT_OBSERVATIONS_ACK,
        FRONT_OBSERVATIONS_STREAM,
    },
    redis::Streams,
    Store,
};

pub(crate) async fn spawn(
    store: Store,
    streams: Streams,
) -> Result<tokio::task::JoinHandle<()>, spc_store::redis::RedisError> {
    streams
        .ensure_group_from(FRONT_OBSERVATIONS_STREAM, "payout-observations", "0")
        .await?;
    Ok(tokio::spawn(async move {
        loop {
            if let Err(error) = pump(&store, &streams).await {
                tracing::warn!(error=%error,"SPc observation persistence unavailable");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }))
}

async fn pump(store: &Store, streams: &Streams) -> anyhow::Result<()> {
    let mut entries = streams
        .read_pending(
            FRONT_OBSERVATIONS_STREAM,
            "payout-observations",
            "writer",
            64,
        )
        .await?;
    if entries.is_empty() {
        entries = streams
            .read_group(
                FRONT_OBSERVATIONS_STREAM,
                "payout-observations",
                "writer",
                16,
            )
            .await?;
    }
    for (id, bytes) in entries {
        match FrontObservation::decode(&bytes) {
            Ok(request) => {
                let observation = match store
                    .record_observation(
                        "front_derivation",
                        &request.session,
                        Some(&request.attempt),
                        &request.body,
                    )
                    .await
                {
                    Ok(id) => id,
                    Err(spc_store::postgres::StoreError::Invalid(reason)) => {
                        tracing::warn!(reason, "conflicting private observation; no receipt");
                        streams
                            .ack(FRONT_OBSERVATIONS_STREAM, "payout-observations", &id)
                            .await?;
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                };
                let ack = serde_json::to_string(&FrontObservationAck {
                    digest: observation_digest(&bytes),
                    observation_id: observation,
                })?;
                streams
                    .hset(FRONT_OBSERVATIONS_ACK, &request.field(), &ack)
                    .await?;
            }
            Err(_) => tracing::warn!(
                bytes = bytes.len(),
                "invalid private SPc observation; no receipt"
            ),
        }
        streams
            .ack(FRONT_OBSERVATIONS_STREAM, "payout-observations", &id)
            .await?;
    }
    Ok(())
}
