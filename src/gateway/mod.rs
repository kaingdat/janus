pub mod proxy;

use std::sync::Arc;

use crate::routing::{Backend, RouteTable};

#[derive(Clone)]
pub struct Gateway {
    pub routes: Arc<RouteTable>,
}

#[derive(Default)]
pub struct RequestCtx {
    pub host: Option<Arc<str>>,
    pub backend: Option<Arc<Backend>>,
}
