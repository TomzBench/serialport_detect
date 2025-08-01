#![deny(clippy::all)]

use futures::prelude::*;
use napi::{
  bindgen_prelude::*, tokio::sync::mpsc::error::TrySendError,
  tokio_stream::wrappers::ReceiverStream, Error, Result,
};
use napi_derive::napi;
use serialport_detect::{AbortHandle, EventInfo, EventIter};
use std::{
  pin::Pin,
  task::{Context, Poll},
};

#[napi]
pub struct Monitor {
  abort: Option<AbortHandle>,
  stream: Option<EventIter>,
}

/// See [`EventIter`]
///
/// A Javascript runtime wrapper around [`EventIter`] which simply maps the error
#[napi]
pub struct JsEventIter {
  inner: EventIter,
}
impl Stream for JsEventIter {
  type Item = Result<EventInfo>;
  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    match futures::ready!(self.inner.poll_next_unpin(cx)) {
      None => Poll::Ready(None),
      Some(Ok(result)) => Poll::Ready(Some(Ok(result))),
      Some(Err(e)) => Poll::Ready(Some(Err(Error::from_reason(e.to_string())))),
    }
  }
}

#[napi]
impl Monitor {
  #[napi(constructor)]
  pub fn new() -> Result<Monitor> {
    // TODO create something to pipe this as a log stream
    use tracing_subscriber::{filter::LevelFilter, fmt, layer::SubscriberExt, prelude::*};
    let stdout = fmt::layer()
      .compact()
      .with_ansi(true)
      .with_level(true)
      .with_file(false)
      .with_line_number(false)
      .with_target(true);
    tracing_subscriber::registry()
      .with(stdout)
      .with(LevelFilter::TRACE)
      .init();

    serialport_detect::listen()
      .map_err(|e| Error::from_reason(e.to_string()))
      .map(|(abort, stream)| Monitor {
        abort: Some(abort),
        stream: Some(stream),
      })
  }

  #[napi]
  pub fn listen(&mut self, env: &Env) -> Result<ReadableStream<EventInfo>> {
    let inner = self
      .stream
      .take()
      .ok_or_else(|| Error::from_reason("Can only call listen once"))?;
    ReadableStream::new(env, JsEventIter { inner })
  }

  #[napi]
  pub fn abort(&mut self) {
    // Drop abort handle, cause abort
    let _abort = self.abort.take();
  }
}

#[napi]
pub fn create_readable_stream(env: &Env) -> Result<ReadableStream<'_, BufferSlice<'_>>> {
  let (tx, rx) = napi::tokio::sync::mpsc::channel(100);
  std::thread::spawn(move || {
    for _ in 0..100 {
      match tx.try_send(Ok(b"hello".to_vec())) {
        Err(TrySendError::Closed(_)) => {
          panic!("closed");
        }
        Err(TrySendError::Full(_)) => {
          panic!("queue is full");
        }
        Ok(_) => {}
      }
    }
  });
  ReadableStream::create_with_stream_bytes(env, ReceiverStream::new(rx))
}
