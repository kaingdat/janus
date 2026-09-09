use std::path::Path;
use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use serde::Deserialize;
use tracing::warn;

use crate::routing::{Backend, BackendPool, DEFAULT_HOST, HostRoutes, RouteTable};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upstreams {
    provider: Provider,
    #[serde(default)]
    upstreams: Option<HashMap<String, HostConfig>>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    File,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    paths: HashMap<String, PathConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathConfig {
    servers: Vec<String>,
}

pub fn load_route_table(path: impl AsRef<Path>) -> anyhow::Result<RouteTable> {
    let path = path.as_ref();
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading upstreams  file {}", path.display()))?;
    let parsed: Upstreams = serde_norway::from_str(&raw)
        .with_context(|| format!("parsing upstreams file {}", path.display()))?;

    match &parsed.provider {
        Provider::File => Ok(build_route_table(parsed.upstreams.as_ref())),
        other => {
            warn!(provider = ?other,   "provider is not yet implemented; starting with an empty route table");
            Ok(RouteTable::new())
        }
    }
}

fn build_route_table(upstreams: Option<&HashMap<String, HostConfig>>) -> RouteTable {
    let table = RouteTable::new();
    let Some(upstreams) = upstreams.filter(|upstreams| !upstreams.is_empty()) else {
        warn!("no upstreams declared; every request will be answered with 502");
        return table;
    };
    for (host, host_config) in upstreams {
        let routes = HostRoutes::new();
        for (path, path_config) in &host_config.paths {
            let mut backends = Vec::with_capacity(path_config.servers.len());
            for server in &path_config.servers {
                match server.parse().ok() {
                    Some(addr) => backends.push(Arc::new(Backend { addr })),
                    None => warn!(
                        host,
                        path, server, "skipping malformed server entry (expected ip:port)"
                    ),
                }
            }
            if backends.is_empty() {
                warn!(
                    host,
                    path,
                    "route has no usable backends; dropping it so requests fall back to a shorter prefix"
                );
                continue;
            }
            routes.insert(
                Arc::from(path.as_str()),
                Arc::new(BackendPool::new(backends)),
            );
        }
        if routes.is_empty() {
            warn!(
                host,
                "host has no usable routes; dropping it so requests fall back to the default host"
            );
            continue;
        }
        let host_key: Arc<str> = if host.eq_ignore_ascii_case(DEFAULT_HOST) {
            Arc::from(DEFAULT_HOST)
        } else {
            Arc::from(host.to_ascii_lowercase().as_str())
        };
        table.insert(host_key, routes);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::resolve;

    fn write_temp(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("janus-{name}-{}.yaml", std::process::id()));
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn load_normalizes_hosts_and_drops_bad_routes() {
        let path = write_temp(
            "upstreams",
            "provider: file\n\
             upstreams:\n\
             \x20 App.Example.COM:\n\
             \x20   paths:\n\
             \x20     \"/\":\n\
             \x20       servers:\n\
             \x20         - \"127.0.0.1:8001\"\n\
             \x20     \"/api\":\n\
             \x20       servers:\n\
             \x20         - \"not-an-address\"\n",
        );

        let table = load_route_table(&path).unwrap();
        assert!(table.contains_key("app.example.com"));

        // The malformed "/api" route is dropped, so it falls back to "/".
        let matched = resolve(&table, "app.example.com", "/api/things").unwrap();
        assert_eq!(matched.addr.port(), 8001);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_skips_unknown_provider() {
        let path = write_temp(
            "upstreams-provider",
            "provider: consul\n\
             upstreams:\n\
             \x20 app.example.com:\n\
             \x20   paths:\n\
             \x20     \"/\":\n\
             \x20       servers:\n\
             \x20         - \"127.0.0.1:8001\"\n",
        );

        let table = load_route_table(&path).unwrap();
        assert!(table.is_empty());

        std::fs::remove_file(&path).ok();
    }
}
