use std::fmt;

use tracing::{debug, trace};

pub mod generate;
pub mod utils;
pub mod versions;

#[cfg(target_arch = "wasm32")]
pub use generate::generate_latest_czip;
pub use generate::{generate_latest, generate_latest_bin, LatestCzip};

pub use crate::versions::v1::CZipV1;

#[derive(Debug)]
pub enum CZipError {
    UnexpectedEof(&'static str),
    InvalidMagic(u8),
    InvalidUtf8 {
        label: &'static str,
        source: std::str::Utf8Error,
    },
    Toml(toml::de::Error),
    TrailingData(usize),
}

impl fmt::Display for CZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CZipError::UnexpectedEof(label) => {
                write!(f, "unexpected end of data while reading {label}")
            }
            CZipError::InvalidMagic(id) => write!(f, "unknown CZip magic identifier: {id}"),
            CZipError::InvalidUtf8 { label, .. } => write!(f, "{label} contains invalid UTF-8"),
            CZipError::Toml(_) => write!(f, "configuration TOML is invalid"),
            CZipError::TrailingData(bytes) => {
                write!(
                    f,
                    "trailing data detected after parsing archive ({bytes} bytes)"
                )
            }
        }
    }
}

impl std::error::Error for CZipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CZipError::InvalidUtf8 { source, .. } => Some(source),
            CZipError::Toml(err) => Some(err),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, CZipError>;

#[derive(Debug, Clone)]
pub enum CZip {
    V1(CZipV1),
}

impl From<CZip> for Vec<u8> {
    fn from(value: CZip) -> Self {
        let mut buffer = Vec::new();
        let id = get_magic_identifier(&value);
        trace!(magic = id, "Serializing CZip archive");
        buffer.push(id);

        match value {
            CZip::V1(inner) => {
                debug!("Encoding CZip V1 payload");
                let payload: Vec<u8> = inner.into();
                buffer.extend_from_slice(&payload);
            }
        }

        buffer
    }
}

fn get_magic_identifier(czip: &CZip) -> u8 {
    match czip {
        CZip::V1(_) => 1,
    }
}

impl TryFrom<&[u8]> for CZip {
    type Error = CZipError;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        let (first, rest) = bytes
            .split_first()
            .ok_or(CZipError::UnexpectedEof("magic identifier"))?;

        match first {
            1 => {
                trace!(magic = *first, "Detected CZip V1 archive");
                let archive = CZipV1::try_from(rest)?;
                Ok(CZip::V1(archive))
            }
            id => Err(CZipError::InvalidMagic(*id)),
        }
    }
}
