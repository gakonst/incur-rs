use std::{fmt, future::Future, pin::Pin, sync::Arc};

use serde_json::Value;

use crate::{Context, InternalResult as Result};

pub(crate) type BoxFuture = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'static>>;
pub(crate) type BoxHandler = Arc<dyn Fn(Context) -> BoxFuture + Send + Sync>;

/// A composable before/after hook around command execution.
pub trait Middleware: Send + Sync + 'static {
    /// Runs this middleware and optionally continues through [`Next`].
    fn handle(&self, context: Context, next: Next) -> BoxFuture;
}

impl<F, Fut> Middleware for F
where
    F: Fn(Context, Next) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value>> + Send + 'static,
{
    fn handle(&self, context: Context, next: Next) -> BoxFuture {
        Box::pin(self(context, next))
    }
}

/// The remainder of a middleware chain.
#[derive(Clone)]
pub struct Next {
    pub(crate) middlewares: Arc<Vec<Arc<dyn Middleware>>>,
    pub(crate) handler: BoxHandler,
    pub(crate) index: usize,
}

impl fmt::Debug for Next {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Next").field("index", &self.index).finish_non_exhaustive()
    }
}

impl Next {
    /// Continues command execution.
    pub fn run(self, context: Context) -> BoxFuture {
        if let Some(middleware) = self.middlewares.get(self.index).cloned() {
            let next = Self { index: self.index + 1, ..self };
            middleware.handle(context, next)
        } else {
            (self.handler)(context)
        }
    }
}
