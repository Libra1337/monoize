use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EndpointKind {
    List = 1,
    Offers = 2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListCursor {
    pub revision: u64,
    pub limit: u16,
    pub filter_digest: [u8; 32],
    pub group_ordinal: u64,
    pub model: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorError {
    Invalid,
    Stale,
}

impl ListCursor {
    pub fn new(
        revision: u64,
        limit: u16,
        filter_digest: [u8; 32],
        group_ordinal: u64,
        model: &str,
    ) -> Result<Self, CursorError> {
        if !(1..=50).contains(&limit)
            || model.is_empty()
            || model.len() > u16::MAX as usize
            || model
                .as_bytes()
                .iter()
                .any(|byte| matches!(*byte, 0x00..=0x1f | 0x7f))
        {
            return Err(CursorError::Invalid);
        }
        let cursor = Self {
            revision,
            limit,
            filter_digest,
            group_ordinal,
            model: model.to_owned(),
        };
        if encoded_ascii_upper_bound(model.len()) > 512 {
            return Err(CursorError::Invalid);
        }
        Ok(cursor)
    }

    pub fn encode(&self, key: &[u8; 32]) -> Result<String, CursorError> {
        let payload = self.payload();
        let mut mac = HmacSha256::new_from_slice(key).map_err(|_| CursorError::Invalid)?;
        mac.update(&payload);
        let signature = mac.finalize().into_bytes();
        let encoded = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(signature)
        );
        if encoded.len() > 512 {
            return Err(CursorError::Invalid);
        }
        Ok(encoded)
    }

    pub fn decode(
        encoded: &str,
        key: &[u8; 32],
        expected_revision: u64,
        expected_limit: u16,
        expected_filter: [u8; 32],
    ) -> Result<Self, CursorError> {
        if encoded.len() > 512 {
            return Err(CursorError::Invalid);
        }
        let (payload, signature) = encoded.split_once('.').ok_or(CursorError::Invalid)?;
        if signature.contains('.') {
            return Err(CursorError::Invalid);
        }
        let payload = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| CursorError::Invalid)?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| CursorError::Invalid)?;
        let mut mac = HmacSha256::new_from_slice(key).map_err(|_| CursorError::Invalid)?;
        mac.update(&payload);
        mac.verify_slice(&signature)
            .map_err(|_| CursorError::Invalid)?;
        let cursor = Self::parse(&payload)?;
        if cursor.limit != expected_limit || cursor.filter_digest != expected_filter {
            return Err(CursorError::Invalid);
        }
        if cursor.revision != expected_revision {
            return Err(CursorError::Stale);
        }
        Ok(cursor)
    }

    fn payload(&self) -> Vec<u8> {
        let model = self.model.as_bytes();
        let mut output = Vec::with_capacity(54 + model.len());
        output.push(1);
        output.push(EndpointKind::List as u8);
        output.extend_from_slice(&self.revision.to_be_bytes());
        output.extend_from_slice(&self.limit.to_be_bytes());
        output.extend_from_slice(&self.filter_digest);
        output.extend_from_slice(&self.group_ordinal.to_be_bytes());
        output.extend_from_slice(&(model.len() as u16).to_be_bytes());
        output.extend_from_slice(model);
        output
    }

    fn parse(payload: &[u8]) -> Result<Self, CursorError> {
        if payload.len() < 54 || payload[0] != 1 || payload[1] != EndpointKind::List as u8 {
            return Err(CursorError::Invalid);
        }
        let revision = u64::from_be_bytes(
            payload[2..10]
                .try_into()
                .map_err(|_| CursorError::Invalid)?,
        );
        let limit = u16::from_be_bytes(
            payload[10..12]
                .try_into()
                .map_err(|_| CursorError::Invalid)?,
        );
        let filter_digest = payload[12..44]
            .try_into()
            .map_err(|_| CursorError::Invalid)?;
        let group_ordinal = u64::from_be_bytes(
            payload[44..52]
                .try_into()
                .map_err(|_| CursorError::Invalid)?,
        );
        let model_len = u16::from_be_bytes(
            payload[52..54]
                .try_into()
                .map_err(|_| CursorError::Invalid)?,
        ) as usize;
        if payload.len() != 54 + model_len {
            return Err(CursorError::Invalid);
        }
        let model = std::str::from_utf8(&payload[54..]).map_err(|_| CursorError::Invalid)?;
        Self::new(revision, limit, filter_digest, group_ordinal, model)
    }
}

pub fn canonical_filter_digest(kind: EndpointKind, filters: &[(u8, &str)]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update([kind as u8]);
    for (tag, value) in filters {
        digest.update([*tag]);
        digest.update((value.len() as u32).to_be_bytes());
        digest.update(value.as_bytes());
    }
    digest.finalize().into()
}

fn encoded_ascii_upper_bound(model_len: usize) -> usize {
    let payload_len = 54 + model_len;
    let payload_base64 = payload_len.div_ceil(3) * 4;
    let signature_base64 = 44;
    payload_base64 + 1 + signature_base64
}
