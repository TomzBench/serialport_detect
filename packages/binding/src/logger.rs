use napi::{
  bindgen_prelude::*,
  threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
  Task,
};
use napi_derive::napi;
use serde_json::Value;
use std::{
  collections::HashMap,
  sync::mpsc::{Receiver, Sender},
};
use tracing::{error, field::Visit, level_filters::LevelFilter, warn, Subscriber};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

struct VisitJsonLike {
  meta: HashMap<String, Value>,
}
impl Visit for VisitJsonLike {
  fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
    self.meta.insert(field.name().to_string(), value.into());
  }

  fn record_i128(&mut self, field: &tracing::field::Field, value: i128) {
    self
      .meta
      .insert(field.name().to_string(), Value::from(value as i64));
  }

  fn record_u128(&mut self, field: &tracing::field::Field, value: u128) {
    self
      .meta
      .insert(field.name().to_string(), Value::from(value as i64));
  }

  fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
    self
      .meta
      .insert(field.name().to_string(), Value::from(value));
  }

  fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
    self
      .meta
      .insert(field.name().to_string(), Value::from(value));
  }

  fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
    self
      .meta
      .insert(field.name().to_string(), Value::from(value));
  }

  fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
    self
      .meta
      .insert(field.name().to_string(), Value::Bool(value));
  }

  fn record_bytes(&mut self, field: &tracing::field::Field, value: &[u8]) {
    self
      .meta
      .insert(field.name().to_string(), Value::from(value.to_vec()));
  }

  fn record_error(
    &mut self,
    field: &tracing::field::Field,
    _value: &(dyn std::error::Error + 'static),
  ) {
    warn!(name = field.name(), "tracing field error ignored")
  }

  fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
    self.meta.insert(
      field.name().to_string(),
      Value::String(format!("{:?}", value)),
    );
  }
}

#[napi(object)]
pub struct LogInfo {
  pub mesg: String,
  pub meta: HashMap<String, Value>,
  pub target: String,
  pub line: Option<u32>,
  pub file: Option<String>,
  pub module_path: Option<String>,
}

pub struct JsTrace {
  tx: Sender<LogMsg>,
}
impl<S> Layer<S> for JsTrace
where
  S: Subscriber,
{
  fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
    let mut visitor = VisitJsonLike {
      meta: HashMap::new(),
    };
    event.record(&mut visitor);
    let mut meta = visitor.meta;
    let mesg = meta
      .remove("message")
      .and_then(|value| value.as_str().map(|s| s.to_string()))
      .unwrap_or_default();
    let result = self.tx.send(LogMsg::Log(LogInfo {
      mesg,
      meta,
      file: event.metadata().file().map(|s| s.to_string()),
      line: event.metadata().line(),
      target: event.metadata().target().to_string(),
      module_path: event.metadata().module_path().map(|s| s.to_string()),
    }));
    if let Err(error) = result {
      error!(?error, "ipc failed to send log event");
    }
  }
}

enum LogMsg {
  Log(LogInfo),
  Abort,
}

pub struct LogTask {
  rx: Receiver<LogMsg>,
  tsfn: ThreadsafeFunction<LogInfo>,
}

impl Task for LogTask {
  type Output = ();
  type JsValue = ();

  // NOTE we can't log in our logging task because it create an infinite loop
  //      We would need another channel if we want to record some errors
  fn compute(&mut self) -> Result<Self::Output> {
    for mesg in self.rx.iter() {
      let _status = match mesg {
        LogMsg::Abort => break,
        LogMsg::Log(info) => self
          .tsfn
          .call(Ok(info), ThreadsafeFunctionCallMode::Blocking),
      };
    }
    Ok(())
  }

  fn resolve(&mut self, _env: Env, _output: Self::Output) -> Result<Self::JsValue> {
    Ok(())
  }

  fn reject(&mut self, _env: Env, _err: Error) -> Result<Self::JsValue> {
    Ok(())
  }

  fn finally(self, _env: Env) -> Result<()> {
    Ok(())
  }
}

/// Handle to a log event transmitter
#[napi]
pub struct Logger {
  /// Used to cancel the remote thread listening to log events
  tx: Sender<LogMsg>,
}

#[napi]
impl Logger {
  #[napi]
  pub fn abort(&self) -> Result<()> {
    self
      .tx
      .send(LogMsg::Abort)
      .map_err(|error| Error::from_reason(error.to_string()))
  }
}

/// Provide event logs to a callback
#[napi]
pub fn configure_logger(tsfn: ThreadsafeFunction<LogInfo>) -> (Logger, AsyncTask<LogTask>) {
  let (tx, rx) = std::sync::mpsc::channel();
  let task = LogTask { rx, tsfn };
  let abort = Logger { tx: tx.clone() };
  tracing_subscriber::registry()
    .with(JsTrace { tx })
    .with(LevelFilter::TRACE)
    .init();
  (abort, AsyncTask::new(task))
}
