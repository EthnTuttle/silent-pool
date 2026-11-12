//! Opaque, nonce-correlated send authorization. No SPc identity/chain fields.
use crate::{
    tagged, ArkSendOrder, Error, Keypair, Message, Reader, Secp256k1, Writer, XOnlyPublicKey,
};

pub const GRANT_LIFETIME_SECS: u64 = 10;
pub const REQUESTS_STREAM: &str = "spc:ark:authorization:requests";
pub const GRANTS_KEY: &str = "spc:ark:authorization:grants";
pub const CONSUMPTION_STREAM: &str = "spc:ark:authorization:consumption";
pub const CONSUMED_KEY: &str = "spc:ark:authorization:consumed";

/// Opaque observation of consuming an existing lease, never another grant.
/// No identity, destination, block hash or membership can enter this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consumption {
    pub grant: AuthorizationGrant,
    pub observed_unix_ms: u64,
}
impl Consumption {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(b"SPAC\x01")
            .bytes(&self.grant.encode())
            .u64(self.observed_unix_ms);
        w.finish()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.array::<5>()? != *b"SPAC\x01" {
            return Err(Error::InvalidTnf);
        }
        let grant = AuthorizationGrant::decode(r.take(149)?)?;
        let observed_unix_ms = r.u64()?;
        r.finish()?;
        Ok(Self {
            grant,
            observed_unix_ms,
        })
    }
    pub fn digest(&self) -> [u8; 32] {
        tagged("SPc/ArkConsumptionV1", &self.encode())
    }
    pub fn field(&self) -> String {
        format!(
            "{}:{}",
            hex::encode(self.grant.request.order_id),
            hex::encode(self.grant.request.nonce)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsumptionAck {
    pub digest: [u8; 32],
    pub observation_id: u64,
    pub recorded_unix_ms: u64,
}
impl ConsumptionAck {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(b"SPCA\x01")
            .bytes(&self.digest)
            .u64(self.observation_id)
            .u64(self.recorded_unix_ms);
        w.finish()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.array::<5>()? != *b"SPCA\x01" {
            return Err(Error::InvalidTnf);
        }
        let result = Self {
            digest: r.array()?,
            observation_id: r.u64()?,
            recorded_unix_ms: r.u64()?,
        };
        r.finish()?;
        if result.observation_id == 0 {
            return Err(Error::InvalidTnf);
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvedReceipt {
    pub grant: AuthorizationGrant,
    pub sats: u64,
    /// Complete stock-protocol VTXOs; every delivered VTXO, including chains.
    pub vtxos: Vec<Vec<u8>>,
}
impl ProvedReceipt {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.vtxos.is_empty() || self.vtxos.len() > 2048 {
            return Err(Error::Length);
        }
        let mut w = Writer::default();
        w.bytes(b"SPRP\x01")
            .bytes(&self.grant.encode())
            .u64(self.sats)
            .u16(self.vtxos.len() as u16);
        for proof in &self.vtxos {
            if proof.is_empty() || proof.len() > 1024 * 1024 {
                return Err(Error::Length);
            }
            w.u32(proof.len() as u32).bytes(proof);
        }
        let bytes = w.finish();
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(Error::Length);
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(Error::Length);
        }
        let mut r = Reader::new(bytes);
        if r.array::<5>()? != *b"SPRP\x01" {
            return Err(Error::InvalidTnf);
        }
        let grant = AuthorizationGrant::decode(r.take(149)?)?;
        let sats = r.u64()?;
        let count = r.u16()? as usize;
        if count == 0 || count > 2048 {
            return Err(Error::Length);
        }
        let mut vtxos = Vec::with_capacity(count);
        for _ in 0..count {
            let len = r.u32()? as usize;
            if len == 0 || len > 1024 * 1024 {
                return Err(Error::Length);
            }
            vtxos.push(r.take(len)?.to_vec());
        }
        r.finish()?;
        Ok(Self { grant, sats, vtxos })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationRequest {
    pub order_id: [u8; 16],
    pub order_digest: [u8; 32],
    pub nonce: [u8; 16],
}

pub fn order_digest(order: &ArkSendOrder) -> Result<[u8; 32], Error> {
    Ok(tagged("SPc/FrozenArkOrderV1", &order.encode()?))
}

impl AuthorizationRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(b"SPAR\x01")
            .bytes(&self.order_id)
            .bytes(&self.order_digest)
            .bytes(&self.nonce);
        w.finish()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.array::<5>()? != *b"SPAR\x01" {
            return Err(Error::InvalidTnf);
        }
        let request = Self {
            order_id: r.array()?,
            order_digest: r.array()?,
            nonce: r.array()?,
        };
        r.finish()?;
        if request.nonce == [0; 16] {
            return Err(Error::InvalidTnf);
        }
        Ok(request)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationGrant {
    pub request: AuthorizationRequest,
    pub revision: u64,
    pub expires_at: u64,
    pub signature: [u8; 64],
}

impl AuthorizationGrant {
    fn payload(&self) -> Vec<u8> {
        let mut w = Writer::default();
        w.bytes(b"SPAG\x01");
        w.bytes(&self.request.order_id)
            .bytes(&self.request.order_digest)
            .bytes(&self.request.nonce)
            .u64(self.revision)
            .u64(self.expires_at);
        w.finish()
    }
    pub fn sign(&mut self, key: &Keypair) {
        let msg = Message::from_digest(tagged("SPc/ArkAuthorizationV1", &self.payload()));
        self.signature = *Secp256k1::signing_only()
            .sign_schnorr_no_aux_rand(&msg, key)
            .as_ref();
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.payload();
        bytes.extend_from_slice(&self.signature);
        bytes
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.array::<5>()? != *b"SPAG\x01" {
            return Err(Error::InvalidTnf);
        }
        let out = Self {
            request: AuthorizationRequest {
                order_id: r.array()?,
                order_digest: r.array()?,
                nonce: r.array()?,
            },
            revision: r.u64()?,
            expires_at: r.u64()?,
            signature: r.array()?,
        };
        r.finish()?;
        Ok(out)
    }
    pub fn verify_for(
        &self,
        request: &AuthorizationRequest,
        now: u64,
        key: &XOnlyPublicKey,
    ) -> Result<(), Error> {
        if &self.request != request
            || self.revision == 0
            || self.expires_at <= now
            || self.expires_at - now > GRANT_LIFETIME_SECS
        {
            return Err(Error::Signature);
        }
        self.verify_signature(key)
    }
    pub fn verify_signature(&self, key: &XOnlyPublicKey) -> Result<(), Error> {
        let sig = bitcoin::secp256k1::schnorr::Signature::from_slice(&self.signature)
            .map_err(|_| Error::Signature)?;
        let msg = Message::from_digest(tagged("SPc/ArkAuthorizationV1", &self.payload()));
        Secp256k1::verification_only()
            .verify_schnorr(&sig, &msg, key)
            .map_err(|_| Error::Signature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grant_is_bound_to_fresh_nonce_order_revision_and_expiry() {
        let sk = bitcoin::secp256k1::SecretKey::from_slice(&[7; 32]).unwrap();
        let key = Keypair::from_secret_key(&Secp256k1::new(), &sk);
        let request = AuthorizationRequest {
            order_id: [1; 16],
            order_digest: [2; 32],
            nonce: [3; 16],
        };
        let mut grant = AuthorizationGrant {
            request: request.clone(),
            revision: 1,
            expires_at: 110,
            signature: [0; 64],
        };
        grant.sign(&key);
        assert!(grant
            .verify_for(&request, 100, &key.x_only_public_key().0)
            .is_ok());
        let mut other = request.clone();
        other.nonce[0] ^= 1;
        assert!(grant
            .verify_for(&other, 100, &key.x_only_public_key().0)
            .is_err());
        assert!(grant
            .verify_for(&request, 110, &key.x_only_public_key().0)
            .is_err());
        grant.revision += 1;
        assert!(grant
            .verify_for(&request, 100, &key.x_only_public_key().0)
            .is_err());
        assert_eq!(
            AuthorizationRequest::decode(&request.encode()).unwrap(),
            request
        );
        let mut bytes = request.encode();
        bytes.push(0);
        assert!(AuthorizationRequest::decode(&bytes).is_err());
        // Fixed-size schema has no identity or member-count field.
        assert_eq!(request.encode().len(), 69);
        assert_eq!(grant.encode().len(), 149);
    }

    #[test]
    fn opaque_consumption_frames_bind_exact_grant_clock_and_refuse_extra_fields() {
        let request = AuthorizationRequest {
            order_id: [1; 16],
            order_digest: [2; 32],
            nonce: [3; 16],
        };
        let grant = AuthorizationGrant {
            request,
            revision: 4,
            expires_at: 110,
            signature: [5; 64],
        };
        let event = Consumption {
            grant,
            observed_unix_ms: 101_234,
        };
        let bytes = event.encode();
        assert_eq!(bytes.len(), 162);
        assert_eq!(Consumption::decode(&bytes).unwrap(), event);
        let mut changed = event.clone();
        changed.observed_unix_ms += 1;
        assert_ne!(changed.digest(), event.digest());
        let ack = ConsumptionAck {
            digest: event.digest(),
            observation_id: 9,
            recorded_unix_ms: 101_240,
        };
        assert_eq!(ack.encode().len(), 53);
        assert_eq!(ConsumptionAck::decode(&ack.encode()).unwrap(), ack);
        let mut extra = bytes.clone();
        extra.extend_from_slice(&[0x91; 32]);
        assert!(Consumption::decode(&extra).is_err());
        for end in 0..bytes.len() {
            assert!(Consumption::decode(&bytes[..end]).is_err());
        }
        assert!(
            !bytes.windows(32).any(|w| w == [0x91; 32]),
            "no identity or block field exists in this frame"
        );
    }
}
