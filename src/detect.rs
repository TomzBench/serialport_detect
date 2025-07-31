// io.rs
#[cfg(unix)]
use crate::posix::EventIter;
use crossbeam::queue::SegQueue;
use futures::Stream;
use parking_lot::Mutex;
use pin_project_lite::pin_project;
use std::collections::HashMap;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use tracing::{trace, warn};

/// Information about the serial port
#[derive(Debug, Clone)]
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
pub enum EventType {
    /// A USB serial port device has been plugged into the system
    Add,
    /// A USB serial port device has been unplugged from the system
    Remove,
}

/// Extra data appended to the event
#[derive(Debug, Clone)]
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
        match self.inner.pop() {
            None => {
                let new_waker = cx.waker();
                let mut waker = self.waker.lock();
                *waker = match waker.take() {
                    None => Some(new_waker.clone()),
                    Some(old_waker) => {
                        if old_waker.will_wake(new_waker) {
                            Some(old_waker)
                        } else {
                            Some(new_waker.clone())
                        }
                    }
                };
                Poll::Pending
            }
            Some(Some(inner)) => Poll::Ready(Some(inner)),
            Some(None) => Poll::Ready(None),
        }
    }
}

pin_project! {
    #[project = DetectProj]
    #[project_replace = DetectProjReplace]
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    /// A Detect is a wrapper around the underlying system event listener. We cache device
    /// information in this object so that when the device is removed, we know the details about
    /// the device that was removed.
    pub enum Detect {
        Streaming {
            #[pin]
            inner: EventIter,
            cache: HashMap<String, EventInfo>
        },
        Cancelled,
        Complete
    }
}

impl Detect {
    pub(crate) fn new() -> io::Result<Detect> {
        // TODO use udev and list some devices
        Ok(Detect::Streaming {
            #[cfg(unix)]
            inner: crate::posix::listen()?,
            #[cfg(unix)]
            cache: crate::posix::scan()?,
        })
    }

    // Stop listening to events
    pub fn cancel(&mut self) {
        match std::mem::replace(self, Detect::Cancelled) {
            Detect::Cancelled => panic!("already cancelled stream!"),
            Detect::Complete => trace!("cancelled a completed stream"),
            Detect::Streaming { .. } => trace!("stream cancelled"),
        }
    }
}

impl Stream for Detect {
    type Item = io::Result<EventInfo>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.as_mut().project() {
                DetectProj::Streaming { inner, cache } => match inner.poll_next(cx) {
                    Poll::Pending => break Poll::Pending,
                    Poll::Ready(None) => {
                        self.project_replace(Self::Complete);
                        break Poll::Ready(None);
                    }
                    Poll::Ready(Some(Err(e))) => break Poll::Ready(Some(Err(e))),
                    Poll::Ready(Some(Ok(ev))) => match ev.event {
                        EventType::Add => {
                            cache.insert(ev.port.clone(), ev.clone());
                            break Poll::Ready(Some(Ok(ev)));
                        }
                        EventType::Remove => match cache.remove(ev.port.as_str()) {
                            None => warn!(port = ev.port.as_str(), "not found in cache"),
                            Some(_info) => break Poll::Ready(Some(Ok(ev))),
                        },
                    },
                },
                DetectProj::Cancelled => break Poll::Ready(None),
                DetectProj::Complete => {
                    panic!("must not be polled after stream has finished")
                }
            }
        }
    }
}
