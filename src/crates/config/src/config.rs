use config::{Config, ConfigBuilder, ConfigError, FileFormat, FileSourceFile};
use std::path::Path;
use walkdir::WalkDir;

pub const ENV_VAR_PREFIX: &str = "CARDINAL";
pub const ENV_VAR_DELIM: &str = "__";

fn get_config_files(
    config_path: &str,
    required: bool,
) -> Vec<config::File<FileSourceFile, FileFormat>> {
    if Path::new(config_path).is_dir() {
        WalkDir::new(config_path)
            .sort_by_file_name()
            .follow_links(true)
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
                    Some(config::File::from(path).required(required))
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![config::File::with_name(config_path).required(required)]
    }
}

pub(crate) fn get_config_builder(
    paths: &[String],
) -> Result<ConfigBuilder<config::builder::DefaultState>, ConfigError> {
    let mut builder = Config::builder();

    for path in paths {
        for file in get_config_files(path.as_str(), true) {
            builder = builder.add_source(file);
        }
    }

    let env_source = config::Environment::with_prefix(ENV_VAR_PREFIX)
        .separator(ENV_VAR_DELIM)
        .list_separator(",")
        .try_parsing(true);

    builder = builder.add_source(env_source);

    Ok(builder)
}
