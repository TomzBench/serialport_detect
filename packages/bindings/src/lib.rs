#![deny(clippy::all)]
pub mod logger;

use futures::prelude::*;
use napi::{
  bindgen_prelude::*,
  threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
  Error, Result,
};
use napi_derive::napi;
use serialport_detect::{AbortHandle, EventInfo};
use tracing::{trace, warn};

#[napi]
pub struct JsAbortHandle {
  inner: Option<AbortHandle>,
}

#[napi]
impl JsAbortHandle {
  #[napi]
  pub fn abort(&mut self) {
    // Drop abort handle, cause abort
    let _abort = self.inner.take();
  }
}

#[napi]
pub fn listen<'env>(
  env: &'env Env,
  tsfn: ThreadsafeFunction<EventInfo>,
) -> Result<(JsAbortHandle, PromiseRaw<'env, ()>)> {
  let (abort, mut stream) =
    serialport_detect::listen().map_err(|e| Error::from_reason(e.to_string()))?;

  let future = env.spawn_future(async move {
    loop {
      let status = match stream.next().await {
        None => break,
        Some(Ok(event)) => tsfn.call(Ok(event), ThreadsafeFunctionCallMode::Blocking),
        Some(Err(e)) => tsfn.call(
          Err(Error::from_reason(e.to_string())),
          ThreadsafeFunctionCallMode::Blocking,
        ),
      };
      match status {
        Status::Ok => trace!("execute threadsafe function"),
        status => warn!(?status, "failed to execute threadsafe function"),
      }
    }
    Ok(())
  })?;
  Ok((JsAbortHandle { inner: Some(abort) }, future))
}
