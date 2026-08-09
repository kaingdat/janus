use anyhow::Context;
use pingora::proxy::http_proxy_service;
use pingora::server::Server;

use crate::gateway::Gateway;

pub fn run() -> anyhow::Result<()> {
    let mut server = Server::new(None).context("failed to create Pingora server")?;
    server.bootstrap();

    let mut proxy = http_proxy_service(&server.configuration, Gateway {});
    proxy.add_tcp("0.0.0.0:6193");

    server.add_service(proxy);
    server.run_forever()
}
