use anyhow::Context;
use pingora::proxy::http_proxy_service;
use pingora::server::configuration::Opt;
use pingora::server::{RunArgs, Server};

use crate::config::main_config::MainConfig;
use crate::gateway::Gateway;
use crate::logging;

pub fn run() -> anyhow::Result<()> {
    let opt = Opt::parse_args();
    let conf_path = opt
        .conf
        .clone()
        .context("no --conf <path> given; a main.yaml path is required")?;

    let config = MainConfig::load(&conf_path)?;
    let logging = logging::init(&config)?;

    let mut server = Server::new(Some(opt)).context("failed to create Pingora server")?;
    server.bootstrap();

    let mut proxy = http_proxy_service(&server.configuration, Gateway {});
    proxy.add_tcp(&config.proxy_address_http);
    server.add_service(proxy);

    let log_guard = logging.attach(&mut server);

    server.run(RunArgs::default());
    log_guard.flush();

    Ok(())
}
