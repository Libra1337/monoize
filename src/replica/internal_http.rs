use bytes::{Bytes, BytesMut};
use reqwest::StatusCode;
use thiserror::Error;

pub(crate) const MAX_INTERNAL_RESPONSE_BYTES: usize = 65_536;

#[derive(Debug, Error)]
pub(crate) enum InternalResponseError {
    #[error("internal response exceeds 65536 bytes")]
    TooLarge,
    #[error("internal response body failed: {0}")]
    Transport(String),
}

pub(crate) async fn read_internal_response(
    mut response: reqwest::Response,
) -> Result<(StatusCode, Bytes), InternalResponseError> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_INTERNAL_RESPONSE_BYTES as u64)
    {
        return Err(InternalResponseError::TooLarge);
    }
    let mut body = BytesMut::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_INTERNAL_RESPONSE_BYTES as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| InternalResponseError::Transport(error.to_string()))?
    {
        if chunk.len() > MAX_INTERNAL_RESPONSE_BYTES - body.len() {
            return Err(InternalResponseError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, body.freeze()))
}
