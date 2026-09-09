use std::borrow::Cow;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;
use pingora::proxy::Session;

#[derive(Debug)]
pub struct Backend {
    pub addr: SocketAddr,
}

#[derive(Debug)]
pub struct BackendPool {
    pub backends: Vec<Arc<Backend>>,
    pub cursor: AtomicUsize,
}

impl BackendPool {
    pub fn new(backends: Vec<Arc<Backend>>) -> Self {
        Self {
            backends,
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn next(&self) -> Option<Arc<Backend>> {
        if self.backends.is_empty() {
            return None;
        }
        let i = self.cursor.fetch_add(1, Ordering::Relaxed) % self.backends.len();
        Some(self.backends[i].clone())
    }
}

pub type HostRoutes = DashMap<Arc<str>, Arc<BackendPool>>;

pub type RouteTable = DashMap<Arc<str>, HostRoutes>;

pub const DEFAULT_HOST: &str = "DEFAULT";

pub fn request_host(session: &Session) -> Option<Cow<'_, str>> {
    let host = if session.is_http2() {
        session.req_header().uri.host()?
    } else {
        let host = session.req_header().headers.get("host")?.to_str().ok()?;
        host.split_once(":").map_or(host, |(host, _)| host)
    };
    let canonical_host = if host.bytes().any(|b| b.is_ascii_uppercase()) {
        Cow::Owned(host.to_ascii_lowercase())
    } else {
        Cow::Borrowed(host)
    };
    Some(canonical_host)
}

pub fn resolve(routes: &RouteTable, host: &str, path: &str) -> Option<Arc<Backend>> {
    let host_routes = routes.get(host).or_else(|| routes.get(DEFAULT_HOST))?;
    let mut slice = path;
    loop {
        if let Some(pool) = host_routes.get(slice) {
            return pool.next();
        }
        match slice.rfind("/") {
            Some(0) if slice.len() > 1 => slice = "/",
            Some(0) | None => return None,
            Some(pos) => slice = &slice[..pos],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(port: u16) -> Arc<BackendPool> {
        Arc::new(BackendPool::new(vec![Arc::new(Backend {
            addr: format!("127.0.0.1:{port}").parse().unwrap(),
        })]))
    }

    fn table(host: &str, routes: &[(&str, u16)]) -> RouteTable {
        let host_routes = HostRoutes::new();
        for (path, port) in routes {
            host_routes.insert(Arc::from(*path), pool(*port));
        }
        let table = RouteTable::new();
        table.insert(Arc::from(host), host_routes);
        table
    }

    #[test]
    fn next_cycles_backends() {
        let backends: Vec<Arc<Backend>> = [8001, 8002]
            .iter()
            .map(|port| {
                Arc::new(Backend {
                    addr: format!("127.0.0.1:{port}").parse().unwrap(),
                })
            })
            .collect();
        let pool = BackendPool::new(backends);

        let ports: Vec<u16> = (0..4).map(|_| pool.next().unwrap().addr.port()).collect();
        assert_eq!(ports, [8001, 8002, 8001, 8002]);
        assert!(BackendPool::new(Vec::new()).next().is_none());
    }

    #[test]
    fn resolve_longest_prefix() {
        let routes = table("app.example.com", &[("/", 8001), ("/api/v1", 8002)]);

        let matched = resolve(&routes, "app.example.com", "/api/v1/users").unwrap();
        assert_eq!(matched.addr.port(), 8002);

        let fell_back = resolve(&routes, "app.example.com", "/static/logo.png").unwrap();
        assert_eq!(fell_back.addr.port(), 8001);
    }

    #[test]
    fn resolve_default_host_fallback() {
        let routes = table(DEFAULT_HOST, &[("/api", 8003)]);

        let matched = resolve(&routes, "unknown.example.com", "/api/things").unwrap();
        assert_eq!(matched.addr.port(), 8003);
        assert!(resolve(&routes, "unknown.example.com", "/other").is_none());
    }
}
