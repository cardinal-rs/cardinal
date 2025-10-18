use std::collections::HashMap;

use toml::Value;

use crate::{CZip, CZipV1};

#[derive(Debug, Clone)]
pub struct LatestCzip {
    config: Value,
    plugins: HashMap<String, Vec<u8>>,
}

impl LatestCzip {
    pub fn new(config: Value, plugins: HashMap<String, Vec<u8>>) -> Self {
        Self { config, plugins }
    }

    pub fn config(&self) -> &Value {
        &self.config
    }

    pub fn plugins(&self) -> &HashMap<String, Vec<u8>> {
        &self.plugins
    }

    pub fn into_parts(self) -> (Value, HashMap<String, Vec<u8>>) {
        (self.config, self.plugins)
    }
}

pub fn generate_latest(opts: LatestCzip) -> CZip {
    let (config, plugins) = opts.into_parts();
    CZip::V1(CZipV1::with_plugins(config, plugins))
}

pub fn generate_latest_bin(opts: LatestCzip) -> Vec<u8> {
    generate_latest(opts).into()
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{generate_latest_bin, LatestCzip};
    use std::collections::HashMap;

    use js_sys::{Array, Object, Reflect, Uint8Array};
    use toml::Value;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    #[wasm_bindgen(js_name = generateLatestCzip)]
    pub fn generate_latest_czip(config_toml: String, plugins: JsValue) -> Result<Vec<u8>, JsValue> {
        let config = toml::from_str::<Value>(&config_toml)
            .map_err(|err| JsValue::from_str(&format!("invalid CZIP configuration: {err}")))?;
        let plugin_map = parse_plugins(plugins)?;
        Ok(generate_latest_bin(LatestCzip::new(config, plugin_map)))
    }

    fn parse_plugins(raw: JsValue) -> Result<HashMap<String, Vec<u8>>, JsValue> {
        if raw.is_null() || raw.is_undefined() {
            return Ok(HashMap::new());
        }

        let object = raw
            .dyn_into::<Object>()
            .map_err(|_| JsValue::from_str("plugins must be a plain object"))?;
        let entries = Object::entries(&object);
        let mut plugins = HashMap::with_capacity(entries.length() as usize);

        for idx in 0..entries.length() {
            let entry = entries.get(idx);
            let entry = Array::from(&entry);
            let name = entry
                .get(0)
                .as_string()
                .ok_or_else(|| JsValue::from_str("plugin names must be strings"))?;
            let value = entry.get(1);
            let bytes = Uint8Array::new(&value).to_vec();
            plugins.insert(name, bytes);
        }

        Ok(plugins)
    }

    #[cfg(test)]
    mod tests {
        use super::generate_latest_czip;
        use crate::{generate_latest_bin, CZip, LatestCzip};
        use js_sys::{Object, Reflect, Uint8Array};
        use std::collections::HashMap;
        use wasm_bindgen::JsValue;
        use wasm_bindgen_test::*;

        #[wasm_bindgen_test]
        fn generates_archive_from_js_inputs() {
            let config = r#"
            [gateway]
            version = "1.0"
            "#
            .to_string();

            let plugins_js = Object::new();
            let payload = Uint8Array::from(&[0xAA, 0xBB, 0xCC][..]);
            Reflect::set(&plugins_js, &JsValue::from_str("logger"), &payload.into())
                .expect("should set plugin payload");

            let bytes = generate_latest_czip(config.clone(), JsValue::from(plugins_js))
                .expect("wasm binding should succeed");

            let archive = CZip::try_from(bytes.as_slice()).expect("archive should deserialize");

            let mut expected_plugins = HashMap::new();
            expected_plugins.insert("logger".to_string(), vec![0xAA, 0xBB, 0xCC]);

            match archive {
                CZip::V1(inner) => {
                    let expected_config: toml::Value =
                        toml::from_str(&config).expect("config should parse for comparison");
                    assert_eq!(inner.config(), &expected_config);
                    assert_eq!(inner.plugins(), &expected_plugins);
                }
            }
        }

        #[wasm_bindgen_test]
        fn js_generation_matches_rust() {
            let config_src = r#"
            [gateway]
            version = "1.0"
            "#;
            let config_value: toml::Value =
                toml::from_str(config_src).expect("config should parse for native generation");

            let mut native_plugins = HashMap::new();
            native_plugins.insert("logger".to_string(), vec![0x01, 0x02, 0x03]);
            native_plugins.insert("metrics".to_string(), vec![0xAA, 0xBB]);

            let native_bytes = generate_latest_bin(LatestCzip::new(
                config_value.clone(),
                native_plugins.clone(),
            ));

            let plugins_js = Object::new();
            for (name, payload) in native_plugins.iter() {
                let array = Uint8Array::from(payload.as_slice());
                Reflect::set(&plugins_js, &JsValue::from_str(name), &array.into())
                    .expect("should map plugin payload into JS object");
            }

            let wasm_bytes =
                generate_latest_czip(config_src.to_string(), JsValue::from(plugins_js))
                    .expect("wasm binding should succeed");

            assert_eq!(
                wasm_bytes, native_bytes,
                "WASM output should match native payload"
            );

            let archive =
                CZip::try_from(wasm_bytes.as_slice()).expect("archive should deserialize");
            match archive {
                CZip::V1(inner) => {
                    assert_eq!(inner.config(), &config_value);
                    assert_eq!(inner.plugins(), &native_plugins);
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::generate_latest_czip;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use toml::Value;

    #[test]
    fn generates_czip_payload() {
        let config: Value = toml::from_str(
            r#"
            [gateway]
            version = "1.0"
            "#,
        )
        .expect("config should parse");

        let mut plugins = HashMap::new();
        plugins.insert("logger".to_string(), vec![1, 2, 3]);

        let bytes = generate_latest_bin(LatestCzip::new(config.clone(), plugins.clone()));
        let archive = crate::CZip::try_from(bytes.as_slice()).expect("archive should deserialize");

        match archive {
            crate::CZip::V1(inner) => {
                assert_eq!(inner.config(), &config);
                assert_eq!(inner.plugins(), &plugins);
            }
        }
    }
}
