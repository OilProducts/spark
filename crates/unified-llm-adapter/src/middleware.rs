use std::sync::Arc;

use crate::errors::AdapterError;
use crate::events::StreamEvents;
use crate::request::{Request, Response};

pub type CompleteNext<'a> = dyn Fn(Request) -> Result<Response, AdapterError> + Send + Sync + 'a;
pub type StreamNext<'a> = dyn Fn(Request) -> Result<StreamEvents, AdapterError> + Send + Sync + 'a;

pub trait Middleware: Send + Sync {
    fn complete(
        &self,
        request: Request,
        next: &CompleteNext<'_>,
    ) -> Result<Response, AdapterError> {
        next(request)
    }

    fn stream(
        &self,
        request: Request,
        next: &StreamNext<'_>,
    ) -> Result<StreamEvents, AdapterError> {
        next(request)
    }
}

pub(crate) fn run_complete_chain<F>(
    request: Request,
    middleware: &[Arc<dyn Middleware>],
    terminal: F,
) -> Result<Response, AdapterError>
where
    F: Fn(Request) -> Result<Response, AdapterError> + Send + Sync,
{
    fn call_at(
        index: usize,
        request: Request,
        middleware: &[Arc<dyn Middleware>],
        terminal: &CompleteNext<'_>,
    ) -> Result<Response, AdapterError> {
        let Some(layer) = middleware.get(index) else {
            return terminal(request);
        };
        let next = |request| call_at(index + 1, request, middleware, terminal);
        layer.complete(request, &next)
    }

    call_at(0, request, middleware, &terminal)
}

pub(crate) fn run_stream_chain<F>(
    request: Request,
    middleware: &[Arc<dyn Middleware>],
    terminal: F,
) -> Result<StreamEvents, AdapterError>
where
    F: Fn(Request) -> Result<StreamEvents, AdapterError> + Send + Sync,
{
    fn call_at(
        index: usize,
        request: Request,
        middleware: &[Arc<dyn Middleware>],
        terminal: &StreamNext<'_>,
    ) -> Result<StreamEvents, AdapterError> {
        let Some(layer) = middleware.get(index) else {
            return terminal(request);
        };
        let next = |request| call_at(index + 1, request, middleware, terminal);
        layer.stream(request, &next)
    }

    call_at(0, request, middleware, &terminal)
}
