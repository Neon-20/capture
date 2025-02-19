use std::collections::HashMap;
use std::io::prelude::*;

use bytes::{Buf, Bytes};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use tracing::instrument;
use uuid::Uuid;

use crate::api::CaptureError;

#[derive(Deserialize, Default)]
pub enum Compression {
    #[default]
    Unsupported,

    #[serde(rename = "gzip", alias = "gzip-js")]
    Gzip,
}

#[derive(Deserialize, Default)]
pub struct EventQuery {
    pub compression: Option<Compression>,

    #[serde(alias = "ver")]
    pub lib_version: Option<String>,

    #[serde(alias = "_")]
    pub sent_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct EventFormData {
    pub data: String,
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct RawEvent {
    #[serde(
        alias = "$token",
        alias = "api_key",
        skip_serializing_if = "Option::is_none"
    )]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distinct_id: Option<String>,
    pub uuid: Option<Uuid>,
    pub event: String,
    #[serde(default)]
    pub properties: HashMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>, // Passed through if provided, parsed by ingestion
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>, // Passed through if provided, parsed by ingestion
    #[serde(rename = "$set", skip_serializing_if = "Option::is_none")]
    pub set: Option<HashMap<String, Value>>,
    #[serde(rename = "$set_once", skip_serializing_if = "Option::is_none")]
    pub set_once: Option<HashMap<String, Value>>,
}

static GZIP_MAGIC_NUMBERS: [u8; 3] = [0x1f, 0x8b, 8];

#[derive(Deserialize)]
#[serde(untagged)]
enum RawRequest {
    /// Batch of events
    Batch(Vec<RawEvent>),
    /// Single event
    One(Box<RawEvent>),
}

impl RawRequest {
    pub fn events(self) -> Vec<RawEvent> {
        match self {
            RawRequest::Batch(events) => events,
            RawRequest::One(event) => vec![*event],
        }
    }
}

impl RawEvent {
    /// Takes a request payload and tries to decompress and unmarshall it into events.
    /// While posthog-js sends a compression query param, a sizable portion of requests
    /// fail due to it being missing when the body is compressed.
    /// Instead of trusting the parameter, we peek at the payload's first three bytes to
    /// detect gzip, fallback to uncompressed utf8 otherwise.
    #[instrument(skip_all)]
    pub fn from_bytes(_query: &EventQuery, bytes: Bytes) -> Result<Vec<RawEvent>, CaptureError> {
        tracing::debug!(len = bytes.len(), "decoding new event");

        let payload = if bytes.starts_with(&GZIP_MAGIC_NUMBERS) {
            let mut d = GzDecoder::new(bytes.reader());
            let mut s = String::new();
            d.read_to_string(&mut s).map_err(|e| {
                tracing::error!("failed to decode gzip: {}", e);
                CaptureError::RequestDecodingError(String::from("invalid gzip data"))
            })?;
            s
        } else {
            String::from_utf8(bytes.into()).map_err(|e| {
                tracing::error!("failed to decode body: {}", e);
                CaptureError::RequestDecodingError(String::from("invalid body encoding"))
            })?
        };

        tracing::debug!(json = payload, "decoded event data");
        Ok(serde_json::from_str::<RawRequest>(&payload)?.events())
    }

    pub fn extract_token(&self) -> Option<String> {
        match &self.token {
            Some(value) => Some(value.clone()),
            None => self
                .properties
                .get("token")
                .and_then(Value::as_str)
                .map(String::from),
        }
    }
}

#[derive(Debug)]
pub struct ProcessingContext {
    pub lib_version: Option<String>,
    pub sent_at: Option<OffsetDateTime>,
    pub token: String,
    pub now: String,
    pub client_ip: String,
}

#[derive(Clone, Default, Debug, Serialize, Eq, PartialEq)]
pub struct ProcessedEvent {
    pub uuid: Uuid,
    pub distinct_id: String,
    pub ip: String,
    pub data: String,
    pub now: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub sent_at: Option<OffsetDateTime>,
    pub token: String,
}

impl ProcessedEvent {
    pub fn key(&self) -> String {
        format!("{}:{}", self.token, self.distinct_id)
    }
}

#[cfg(test)]
mod tests {
    use super::Compression;
    use base64::Engine as _;
    use bytes::Bytes;

    use super::{EventQuery, RawEvent};

    #[test]
    fn decode_bytes() {
        let horrible_blob = "H4sIAAAAAAAAA31T207cMBD9lSrikSy+5bIrVX2g4oWWUlEqBEKRY08Sg4mD4+xCEf/e8XLZBSGeEp+ZOWOfmXPxkMAS+pAskp1BtmBBLiHZTQbvBvDBwJgsHpIdh5/kp1Rffp18OcMwAtUS/GhcjwFKZjSbkYjX3q1G8AgeGA+Nu4ughqVRUIX7ATDwHcbr4IYYUJP32LyavMVAF8Kw2NuzTknbuTEsSkIIHlvTf+vhLnzdizUxgslvs2JgkKHr5U1s8VS0dZ/NZSnlW7CVfTvhs7EG+vT0JJaMygP0VQem7bDTvBAbcGV06JAkIwTBpYHV4Hx4zS1FJH+FX7IFj7A1NbZZQR2b4GFbwFlWzFjETY/XCpXRiN538yt/S9mdnm7bSa+lDCY+kOalKDJGs/msZMVuos0YTK+e62hZciHqes7LnDcpoVmTg+TAaqnKMhWUaaa4TllBoCDpJn2uYK3k87xeyFjZFHWdzxmdq5Q0IstBzRXlDMiHbM/5kgnerKfs+tFZqHAolQflvDZ9W0Evawu6wveiENVoND4s+Ami2jBGZbayn/42g3xblizX4skp4FYMYfJQoSQf8DfSjrGBVMEsoWpArpMbK1vc8ItLDG1j1SDvrZM6muBxN/Eg7U1cVFw70KmyRl13bhqjYeBGGrtuFqWTSzzF/q8tRyvV9SfxHXQLoBuidXY0ekeF+KQnNCqgHXaIy7KJBncNERk6VUFhhB33j8zv5uhQ/rCTvbq9/9seH5Pj3Bf/TsuzYf9g2j+3h9N6yZ8Vfpmx4KSguSY5S0lOqc5LmgmhidoMmOaixoFvktFKOo9kK9Nrt3rPxViWk5RwIhtJykZzXohP2DjmZ08+bnH/4B1fkUnGSp2SMmNlIYTguS5ga//eERZZTSVeD8cWPTMGeTMgHSOMpyRLGftDyUKwBV9b6Dx5vPwPzQHjFwsFAAA=";
        let decoded_horrible_blob = base64::engine::general_purpose::STANDARD
            .decode(horrible_blob)
            .unwrap();

        let bytes = Bytes::from(decoded_horrible_blob);
        let events = RawEvent::from_bytes(
            &EventQuery {
                compression: Some(Compression::Gzip),
                lib_version: None,
                sent_at: None,
            },
            bytes,
        );

        assert!(events.is_ok());
    }
}
