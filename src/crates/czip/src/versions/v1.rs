use std::collections::HashMap;
use std::convert::TryFrom;
use toml::Value;

use crate::utils::bytes::{read_exact, read_u32};
use crate::{CZipError, Result};
use tracing::{debug, trace};

#[derive(Debug, Clone)]
pub struct CZipV1 {
    config: Value,
    plugins: HashMap<String, Vec<u8>>,
}

impl CZipV1 {
    /// Creates a CZip V1 archive with the provided configuration and no plugins.
    pub fn new(config: Value) -> Self {
        Self {
            config,
            plugins: HashMap::new(),
        }
    }

    /// Creates a CZip V1 archive from an existing plugin map.
    pub fn with_plugins(config: Value, plugins: HashMap<String, Vec<u8>>) -> Self {
        Self { config, plugins }
    }

    /// Adds or replaces a plugin payload by name.
    pub fn add_plugin<S: Into<String>>(&mut self, name: S, payload: Vec<u8>) {
        self.plugins.insert(name.into(), payload);
    }

    pub fn config(&self) -> &Value {
        &self.config
    }

    pub fn plugins(&self) -> &HashMap<String, Vec<u8>> {
        &self.plugins
    }
}

// Binary layout (little-endian):
// [config_len:u32][config_toml_bytes][plugin_count:u32]
//   repeated { [name_len:u32][name_bytes][payload_len:u32][payload_bytes] }
impl From<CZipV1> for Vec<u8> {
    fn from(value: CZipV1) -> Self {
        let mut buffer = Vec::new();
        trace!("Encoding configuration TOML for CZip V1");

        let config_str =
            toml::to_string(&value.config).expect("failed to serialize CZip configuration to TOML");
        let config_bytes = config_str.as_bytes();
        let config_len = u32::try_from(config_bytes.len())
            .expect("configuration payload exceeds u32::MAX bytes");
        buffer.extend_from_slice(&config_len.to_le_bytes());
        buffer.extend_from_slice(config_bytes);

        let mut plugins: Vec<(String, Vec<u8>)> = value.plugins.into_iter().collect();
        plugins.sort_by(|(left, _), (right, _)| left.cmp(right));
        let plugin_count_u64 = plugins.len();
        debug!(plugin_count = plugin_count_u64, names = ?plugins.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(), "Serializing plugins");
        let plugin_count =
            u32::try_from(plugin_count_u64).expect("plugin count exceeds u32::MAX entries");
        buffer.extend_from_slice(&plugin_count.to_le_bytes());

        for (name, payload) in plugins {
            trace!(plugin = %name, payload_len = payload.len(), "Writing plugin entry");
            let name_bytes = name.as_bytes();
            let name_len =
                u32::try_from(name_bytes.len()).expect("plugin name exceeds u32::MAX bytes");
            buffer.extend_from_slice(&name_len.to_le_bytes());
            buffer.extend_from_slice(name_bytes);

            let payload_len =
                u32::try_from(payload.len()).expect("plugin payload exceeds u32::MAX bytes");
            buffer.extend_from_slice(&payload_len.to_le_bytes());
            buffer.extend_from_slice(&payload);
        }

        buffer
    }
}

impl TryFrom<&[u8]> for CZipV1 {
    type Error = CZipError;

    fn try_from(bytes: &[u8]) -> Result<Self> {
        let mut cursor = 0usize;

        trace!(total_bytes = bytes.len(), "Decoding CZip V1 archive");

        let config_len = read_u32(bytes, &mut cursor, "config length")? as usize;
        let config_bytes = read_exact(bytes, &mut cursor, config_len, "config bytes")?;
        let config_str =
            std::str::from_utf8(config_bytes).map_err(|source| CZipError::InvalidUtf8 {
                label: "config",
                source,
            })?;
        let config = toml::from_str::<Value>(config_str).map_err(CZipError::Toml)?;

        let plugin_count = read_u32(bytes, &mut cursor, "plugin count")? as usize;
        debug!(plugin_count, "Decoding plugin entries");
        let mut plugins = HashMap::with_capacity(plugin_count);

        for _ in 0..plugin_count {
            let name_len = read_u32(bytes, &mut cursor, "plugin name length")? as usize;
            let name_bytes = read_exact(bytes, &mut cursor, name_len, "plugin name")?;
            let name = std::str::from_utf8(name_bytes)
                .map_err(|source| CZipError::InvalidUtf8 {
                    label: "plugin name",
                    source,
                })?
                .to_owned();

            let payload_len = read_u32(bytes, &mut cursor, "plugin payload length")? as usize;
            let payload = read_exact(bytes, &mut cursor, payload_len, "plugin payload")?.to_vec();

            trace!(plugin = %name, payload_len, "Plugin decoded");
            plugins.insert(name, payload);
        }

        if cursor != bytes.len() {
            return Err(CZipError::TrailingData(bytes.len() - cursor));
        }

        Ok(Self { config, plugins })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CZip, CZipError};
    use std::collections::HashMap;
    use toml::value::Table;
    use toml::Value;

    #[test]
    fn round_trip_serialization() {
        let config = {
            let mut table = Table::new();
            table.insert("title".to_string(), Value::String("Example".to_string()));
            Value::Table(table)
        };

        let mut plugins = HashMap::new();
        plugins.insert("logger".to_string(), vec![0xAA, 0xBB, 0xCC]);
        plugins.insert("metrics".to_string(), vec![0x01, 0x02]);
        let expected_plugins = plugins.clone();

        let archive = CZip::V1(CZipV1::with_plugins(config.clone(), plugins));
        let bytes: Vec<u8> = archive.clone().into();

        let decoded = CZip::try_from(bytes.as_slice()).expect("failed to deserialize archive");

        let decoded_v1 = match decoded {
            CZip::V1(inner) => inner,
        };

        assert_eq!(decoded_v1.config(), &config);
        assert_eq!(decoded_v1.plugins(), &expected_plugins);
    }

    #[test]
    fn truncated_payload_errors() {
        let bytes = vec![1, 0x04, 0x00, 0x00, 0x00, b'{', b'}'];

        let err = CZip::try_from(bytes.as_slice()).expect_err("expected truncated payload to fail");
        assert!(matches!(err, CZipError::UnexpectedEof("config bytes")));
    }

    #[test]
    fn trailing_data_is_rejected() {
        let config = {
            let mut table = Table::new();
            table.insert("title".to_string(), Value::String("Example".to_string()));
            Value::Table(table)
        };
        let archive = CZip::V1(CZipV1::new(config));
        let mut bytes: Vec<u8> = archive.into();
        bytes.push(0xFF);

        let err = CZip::try_from(bytes.as_slice()).expect_err("expected trailing data to fail");
        assert!(matches!(err, CZipError::TrailingData(1)));
    }
}
