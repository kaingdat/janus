use anyhow::Context;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct MainConfig {
    pub proxy_address_http: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub log_file: Option<String>,
    #[serde(default)]
    pub master_key: Option<String>,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl std::fmt::Debug for MainConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainConfig")
            .field("proxy_address_http", &self.proxy_address_http)
            .field("log_level", &self.log_level)
            .field("log_file", &self.log_file)
            .field(
                "master_key",
                &self.master_key.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl MainConfig {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading main config {}", path.display()))?;
        let mut config: MainConfig = serde_norway::from_str(&raw)
            .with_context(|| format!("parsing main config {}", path.display()))?;

        config.apply_key_override(std::env::var("JWT_KEY").ok());

        Ok(config)
    }

    fn apply_key_override(&mut self, from_env: Option<String>) {
        match from_env {
            Some(key) if !key.trim().is_empty() => self.master_key = Some(key),
            Some(_) => {
                eprintln!("JWT_KEY is set but empty; keeping master_key from the config file")
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("janus-{name}-{}.yaml", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn load_reads_and_parses_a_config_file() {
        let path = write_temp(
            "parse",
            "proxy_address_http: 0.0.0.0:7000\nlog_level: debug\n",
        );

        let cfg = MainConfig::load(&path).unwrap();
        assert_eq!(cfg.proxy_address_http, "0.0.0.0:7000");
        assert_eq!(cfg.log_level, "debug");

        std::fs::remove_file(&path).ok();
    }
}
