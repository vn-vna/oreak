use std::io::{Read, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use thiserror::Error;

pub const DATA_CODEC_SALT_SIZE: usize = 8;

/// Decoded DataCodec bytes and the complete salt, including any metadata suffix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedData {
    pub data: Vec<u8>,
    pub salt: Vec<u8>,
}

/// Unity's `salt:data:CRC32` codec.
pub struct DataCodec;

impl DataCodec {
    pub fn decode(encoded: &str) -> Result<DecodedData, CodecError> {
        decode_inner(encoded, false, None, false)
    }

    pub fn decode_gzip(encoded: &str, expected_len: usize) -> Result<DecodedData, CodecError> {
        decode_inner(encoded, true, Some(expected_len), false)
    }

    pub fn encode(data: &[u8]) -> Result<String, CodecError> {
        Self::encode_gzip_if(data, false)
    }

    pub fn encode_gzip(data: &[u8]) -> Result<String, CodecError> {
        Self::encode_gzip_if(data, true)
    }

    pub fn encode_with_salt(data: &[u8], salt: &[u8], gzip: bool) -> Result<String, CodecError> {
        if salt.len() < DATA_CODEC_SALT_SIZE {
            return Err(CodecError::SaltTooShort { actual: salt.len() });
        }
        let payload = if gzip { compress(data)? } else { data.to_vec() };
        Ok(encode_payload(&payload, salt))
    }

    fn encode_gzip_if(data: &[u8], gzip: bool) -> Result<String, CodecError> {
        let mut salt = [0_u8; DATA_CODEC_SALT_SIZE];
        getrandom::fill(&mut salt).map_err(|_| CodecError::Random)?;
        Self::encode_with_salt(data, &salt, gzip)
    }
}

pub(crate) fn decode_grid(encoded: &str) -> Result<DecodedData, CodecError> {
    decode_inner(encoded, false, None, true)
}

pub(crate) fn encode_with_metadata(data: &[u8], metadata: &[u8]) -> Result<String, CodecError> {
    let mut salt = vec![0_u8; DATA_CODEC_SALT_SIZE + metadata.len()];
    getrandom::fill(&mut salt[..DATA_CODEC_SALT_SIZE]).map_err(|_| CodecError::Random)?;
    salt[DATA_CODEC_SALT_SIZE..].copy_from_slice(metadata);
    DataCodec::encode_with_salt(data, &salt, false)
}

fn decode_inner(
    encoded: &str,
    gzip: bool,
    expected_len: Option<usize>,
    allow_extended_salt: bool,
) -> Result<DecodedData, CodecError> {
    if encoded.is_empty() {
        return Err(CodecError::Empty);
    }
    let parts: Vec<_> = encoded.split(':').collect();
    if parts.len() != 3 {
        return Err(CodecError::Framing);
    }
    let salt = STANDARD
        .decode(parts[0])
        .map_err(|_| CodecError::Base64 { segment: "salt" })?;
    let obfuscated = STANDARD
        .decode(parts[1])
        .map_err(|_| CodecError::Base64 { segment: "data" })?;
    if salt.len() < DATA_CODEC_SALT_SIZE {
        return Err(CodecError::SaltTooShort { actual: salt.len() });
    }
    if !allow_extended_salt && salt.len() != DATA_CODEC_SALT_SIZE {
        return Err(CodecError::SaltLength { actual: salt.len() });
    }
    let actual = checksum(&salt, &obfuscated);
    if parts[2] != actual {
        return Err(CodecError::Checksum {
            expected: parts[2].to_owned(),
            actual,
        });
    }
    let payload = xor(&obfuscated, &salt);
    let data = if gzip {
        decompress(
            &payload,
            expected_len.ok_or(CodecError::MissingExpectedLength)?,
        )?
    } else {
        payload
    };
    Ok(DecodedData { data, salt })
}

fn encode_payload(payload: &[u8], salt: &[u8]) -> String {
    let obfuscated = xor(payload, salt);
    format!(
        "{}:{}:{}",
        STANDARD.encode(salt),
        STANDARD.encode(&obfuscated),
        checksum(salt, &obfuscated)
    )
}

fn xor(data: &[u8], salt: &[u8]) -> Vec<u8> {
    data.iter()
        .zip(salt.iter().cycle())
        .map(|(byte, salt)| byte ^ salt)
        .collect()
}

fn compress(data: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).map_err(|_| CodecError::Gzip)?;
    encoder.finish().map_err(|_| CodecError::Gzip)
}

fn decompress(payload: &[u8], expected_len: usize) -> Result<Vec<u8>, CodecError> {
    let limit = u64::try_from(expected_len)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut decoder = GzDecoder::new(payload).take(limit);
    let mut data = Vec::with_capacity(expected_len.min(64 * 1024));
    decoder
        .read_to_end(&mut data)
        .map_err(|_| CodecError::Gzip)?;
    if data.len() != expected_len {
        return Err(CodecError::DecodedLength {
            expected: expected_len,
            actual: data.len(),
        });
    }
    Ok(data)
}

fn checksum(salt: &[u8], data: &[u8]) -> String {
    let mut crc = 0xffff_ffff_u32;
    for byte in salt.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xedb8_8320_u32 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    format!("{:08X}", crc ^ 0xffff_ffff_u32)
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("the encoded value is empty")]
    Empty,
    #[error("expected salt:data:checksum")]
    Framing,
    #[error("the {segment} Base64 segment is malformed")]
    Base64 { segment: &'static str },
    #[error("the salt must contain at least {DATA_CODEC_SALT_SIZE} bytes, got {actual}")]
    SaltTooShort { actual: usize },
    #[error("the salt must contain exactly {DATA_CODEC_SALT_SIZE} bytes, got {actual}")]
    SaltLength { actual: usize },
    #[error("the checksum does not match (wire {expected}, computed {actual})")]
    Checksum { expected: String, actual: String },
    #[error("the compressed payload is not valid GZip data")]
    Gzip,
    #[error("the decoded payload contains {actual} bytes; expected {expected}")]
    DecodedLength { expected: usize, actual: usize },
    #[error("a decompressed payload requires an expected length")]
    MissingExpectedLength,
    #[error("the operating system random source is unavailable")]
    Random,
}
