//! Integer absolute planned slots. Restarts reuse the measured boundary and
//! recover the sequence from ALL ordinary command ids in the existing journal.
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const COMMAND_PREFIX: &str = "ordinary/";
pub struct Slot {
    pub sequence: u64,
    pub planned_ms: u64,
    pub entropy: [u8; 32],
}
pub struct Schedule {
    seed: u64,
    driver: String,
    interval_ms: u64,
    start_ms: u64,
    next_sequence: u64,
}
impl Schedule {
    pub fn restore(
        seed: u64,
        driver: String,
        interval_ms: u64,
        start_ms: u64,
        snapshot: &crate::journal::RecoverySnapshot,
    ) -> std::io::Result<Self> {
        if interval_ms == 0 {
            return Err(crate::control::invalid("zero traffic interval"));
        }
        let mut last = 0;
        for id in snapshot
            .commands
            .keys()
            .filter(|id| id.starts_with(COMMAND_PREFIX))
        {
            let n = id[COMMAND_PREFIX.len()..]
                .parse::<u64>()
                .map_err(|_| crate::control::invalid("invalid durable ordinary sequence"))?;
            last = last.max(n);
        }
        Ok(Self {
            seed,
            driver,
            interval_ms,
            start_ms,
            next_sequence: last
                .checked_add(1)
                .ok_or_else(|| crate::control::invalid("schedule exhausted"))?,
        })
    }
    pub fn next_slot(&mut self, now_ms: u64) -> std::io::Result<Option<Slot>> {
        // Downtime/pauses do not accumulate a catch-up burst. Skipped slots
        // have no command/input reservation, while every attempted slot has a
        // durable CommandBegin (including deterministic deferrals).
        let elapsed = now_ms
            .checked_sub(self.start_ms)
            .ok_or_else(|| crate::control::invalid("measured clock moved backwards"))?;
        let sequence = self.next_sequence.max(elapsed / self.interval_ms);
        let mut key = Vec::new();
        key.extend(self.seed.to_be_bytes());
        key.extend(1u32.to_be_bytes());
        key.extend((self.driver.len() as u32).to_be_bytes());
        key.extend(self.driver.as_bytes());
        key.extend(sequence.to_be_bytes());
        let mut hash = Hmac::<Sha256>::new_from_slice(&key)
            .map_err(|_| crate::control::invalid("schedule HMAC key"))?;
        hash.update(b"spc-lab/traffic/planned-schedule");
        let entropy: [u8; 32] = hash.finalize().into_bytes().into();
        let span = self.interval_ms / 10;
        let jitter = u64::from_be_bytes(entropy[..8].try_into().unwrap())
            % (span
                .checked_mul(2)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| crate::control::invalid("jitter overflow"))?);
        let base = self
            .start_ms
            .checked_add(
                sequence
                    .checked_mul(self.interval_ms)
                    .ok_or_else(|| crate::control::invalid("schedule overflow"))?,
            )
            .ok_or_else(|| crate::control::invalid("schedule overflow"))?;
        let planned_ms = base
            .checked_sub(span)
            .and_then(|n| n.checked_add(jitter))
            .ok_or_else(|| crate::control::invalid("schedule overflow"))?;
        if now_ms < planned_ms {
            return Ok(None);
        }
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| crate::control::invalid("schedule exhausted"))?;
        Ok(Some(Slot {
            sequence,
            planned_ms,
            entropy,
        }))
    }
}
