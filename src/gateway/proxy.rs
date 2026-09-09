use async_trait::async_trait;
use pingora::proxy::{ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;
use pingora::{Error, ErrorType, Result};
use tracing::error;

use crate::routing::{DEFAULT_HOST, request_host, resolve};

use super::{Gateway, RequestCtx};

#[async_trait]
impl ProxyHttp for Gateway {
    type CTX = RequestCtx;

    fn new_ctx(&self) -> Self::CTX {
        RequestCtx::default()
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let Some(host) = request_host(session) else {
            return Ok(false);
        };
        let host_key = self
            .routes
            .get(host.as_ref())
            .or_else(|| self.routes.get(DEFAULT_HOST))
            .map(|entry| entry.key().clone());
        if let Some(host_key) = host_key {
            let path = session.req_header().uri.path();
            ctx.backend = resolve(&self.routes, &host_key, path);
            ctx.host = Some(host_key);
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        match (ctx.host.as_ref(), ctx.backend.as_ref()) {
            (Some(host), Some(backend)) => Ok(Box::new(HttpPeer::new(
                backend.addr,
                false,
                host.to_string(),
            ))),
            _ => {
                if let Err(e) = session.respond_error(502).await {
                    error!("failed to send the 502 response: {e:?}");
                }
                Err(Error::explain(
                    ErrorType::HTTPStatus(502),
                    "no upstream for request",
                ))
            }
        }
    }
}
