// io.rs
#[cfg(unix)]
use crossbeam::queue::SegQueue;
use parking_lot::Mutex;
use std::{
    io,
    task::{Context, Poll, Waker},
};
use tracing::trace;

/// Information about the serial port
#[derive(Debug, Clone)]
#[cfg_attr(feature = "napi", napi_derive::napi(object))]
pub struct DeviceInfo {
    /// Vendor ID
    pub vid: Option<String>,
    /// Product ID
    pub pid: Option<String>,
    /// Serial number
    pub serial: Option<String>,
    /// Manufacturer string (arbitrary string)
    pub manufacturer: Option<String>,
    /// Product string (arbitrary string)
    pub product: Option<String>,
}

/// A USB Add or Remove event has occured
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "napi", napi_derive::napi)]
pub enum EventType {
    /// A USB serial port device has been plugged into the system
    Add,
    /// A USB serial port device has been unplugged from the system
    Remove,
}

/// Extra data appended to the event
#[derive(Debug, Clone)]
#[cfg_attr(feature = "napi", napi_derive::napi(object))]
pub struct EventInfo {
    /// The port name, ie COM3 or tty/ACM0
    pub port: String,
    /// Meta data about the port
    pub meta: DeviceInfo,
    /// See [`EventType`]
    pub event: EventType,
}

#[derive(Default)]
pub(crate) struct Queue {
    inner: SegQueue<Option<io::Result<EventInfo>>>,
    waker: Mutex<Option<Waker>>,
}

impl Queue {
    pub(crate) fn new() -> Queue {
        Queue {
            inner: SegQueue::new(),
            waker: Mutex::new(None),
        }
    }

    fn maybe_wake(&self) {
        if let Some(waker) = &self.waker.lock().as_ref() {
            waker.wake_by_ref();
        }
    }

    pub(crate) fn push(&self, ev: io::Result<EventInfo>) {
        self.inner.push(Some(ev));
        self.maybe_wake();
    }

    pub(crate) fn done(&self) {
        self.inner.push(None);
        self.maybe_wake();
    }

    pub(crate) fn poll_next(&self, cx: &mut Context<'_>) -> Poll<Option<io::Result<EventInfo>>> {
        // Waker accounting
        let new_waker = cx.waker();
        let mut waker = self.waker.lock();
        *waker = match waker.take() {
            Some(old_waker) if old_waker.will_wake(new_waker) => Some(old_waker),
            None | Some(_) => Some(new_waker.clone()),
        };

        trace!(remaining = self.inner.len(), "polling");
        match self.inner.pop() {
            None => Poll::Pending,
            Some(Some(inner)) => Poll::Ready(Some(inner)),
            Some(None) => Poll::Ready(None),
        }
    }
}
