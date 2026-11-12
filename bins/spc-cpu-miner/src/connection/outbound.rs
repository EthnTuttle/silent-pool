//! One task owns a connection's encrypt/write state for its entire lifetime.
//! Enqueue never waits for the socket. A full queue or interrupted write tears
//! down the connection; a partially written Noise stream is never reused.
use super::*;
use std::future::Future;
use stratum_core::framing_sv2::framing::EncodableFrame;

pub(super) const CAPACITY: usize = 64;
const MAX_FRAME_BYTES: usize = 1024;
const CLOSE_DEADLINE: Duration = Duration::from_millis(500);
pub(super) type OwnedFrame = stratum_core::framing_sv2::framing::SerializedFrame<Vec<u8>>;

pub(super) trait FrameWriter: Send + 'static {
    fn write(&mut self, frame: OwnedFrame) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn close(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send;
}

impl FrameWriter for NoiseTcpWriteHalf {
    async fn write(&mut self, frame: OwnedFrame) -> anyhow::Result<()> {
        self.write_frame(frame)
            .await
            .map_err(|e| anyhow!(e.to_string()))
    }
    async fn close(&mut self) -> anyhow::Result<()> {
        self.shutdown().await.map_err(|e| anyhow!(e.to_string()))
    }
}

enum Outbound {
    Frame(OwnedFrame),
    Close(OwnedFrame),
}

pub(super) struct Writer {
    tx: mpsc::Sender<Outbound>,
    task: JoinHandle<anyhow::Result<()>>,
}
impl Writer {
    pub(super) fn new(mut transport: impl FrameWriter) -> Self {
        let (tx, mut rx) = mpsc::channel(CAPACITY);
        let task = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                match message {
                    Outbound::Frame(frame) => transport.write(frame).await?,
                    Outbound::Close(frame) => {
                        transport.write(frame).await?;
                        return transport.close().await;
                    }
                }
            }
            Ok(())
        });
        Self { tx, task }
    }

    pub(super) fn send<T>(&self, message: T, message_type: u8, channel: bool) -> anyhow::Result<()>
    where
        T: binary_sv2::Serialize + binary_sv2::GetSize,
    {
        let frame = owned_frame(message, message_type, channel)?;
        self.tx
            .try_send(Outbound::Frame(frame))
            .map_err(|_| anyhow!("native outbound queue full or closed; discard connection"))
    }

    pub(super) async fn finished(&mut self) -> anyhow::Result<()> {
        (&mut self.task)
            .await
            .map_err(|e| anyhow!("native writer task: {e}"))?
    }

    /// Caller has already revoked work and cancelled approval. On timeout,
    /// abort AND join the sole writer owner, discarding its socket/cipher state.
    /// No future write/shutdown operation may run on that transport afterward.
    pub(super) async fn close(&mut self, channel_id: u32) {
        let frame = owned_frame(
            CloseChannel {
                channel_id,
                reason_code: str0("miner-stop").expect("constant close reason"),
            },
            MESSAGE_TYPE_CLOSE_CHANNEL,
            CHANNEL_BIT_CLOSE_CHANNEL,
        )
        .expect("bounded close frame");
        if self.tx.try_send(Outbound::Close(frame)).is_err()
            || timeout(CLOSE_DEADLINE, &mut self.task).await.is_err()
        {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn owned_frame<T>(message: T, message_type: u8, channel: bool) -> anyhow::Result<OwnedFrame>
where
    T: binary_sv2::Serialize + binary_sv2::GetSize,
{
    let frame = MessageFrame::from_message(message, message_type, 0, channel)
        .map_err(|e| anyhow!(e.to_string()))?;
    if frame.encoded_length() > MAX_FRAME_BYTES {
        return Err(anyhow!("native outbound frame bound"));
    }
    let mut bytes = vec![0; frame.encoded_length()];
    frame
        .encode_into(&mut bytes)
        .map_err(|e| anyhow!(e.to_string()))?;
    OwnedFrame::from_bytes(bytes).map_err(|e| anyhow!(e.to_string()))
}

#[cfg(test)]
pub(super) fn queued(writer: &Writer) -> usize {
    CAPACITY - writer.tx.capacity()
}
