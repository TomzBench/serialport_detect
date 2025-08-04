// Posix support

use crate::detect::{DeviceInfo, EventInfo, EventType, Queue};
use futures::Stream;
use mio::{unix::SourceFd, Events, Interest, Token};
use nix::{
    sys::eventfd::{EfdFlags, EventFd},
    unistd,
};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt::{self, Debug},
    io,
    os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    thread::JoinHandle,
};
use tracing::{error, trace};
use udev::Device;

#[derive(Debug)]
struct ListenerOptions {
    capacity: usize,
    evfd: RawFd,
}

/// Scan for connected devices
pub fn scan() -> io::Result<HashMap<String, DeviceInfo>> {
    let mut enumerator = udev::Enumerator::new()?;
    enumerator.match_subsystem("tty")?;
    let items = enumerator
        .scan_devices()?
        .map(|dev| {
            let port = match dev.devnode() {
                Some(path) => path.to_str().unwrap_or("").to_string(),
                _ => "".to_string(),
            };
            (port.clone(), read_device_info(port, &dev))
        })
        .collect();
    Ok(items)
}

/// Listen for connected devices
pub fn listen() -> io::Result<(AbortHandle, EventIter)> {
    let queue = Arc::new(Queue::new());
    let theirs = Arc::clone(&queue);
    let evfd = EventFd::from_value_and_flags(0, EfdFlags::EFD_NONBLOCK | EfdFlags::EFD_SEMAPHORE)?;
    let opts = ListenerOptions {
        capacity: 1024,
        evfd: evfd.as_raw_fd(),
    };
    let join_handle = Some(std::thread::spawn(move || listener(theirs, opts)));
    Ok((AbortHandle { evfd, join_handle }, EventIter { queue }))
}

fn listener(queue: Arc<Queue>, opts: ListenerOptions) {
    // Get a udev socket
    trace!(capacity = opts.capacity, "listening");
    // Safety: EventFd is private and when dropped we close, and remains open until join is called.
    // See EventIter drop
    let evfd = unsafe { BorrowedFd::borrow_raw(opts.evfd) };
    let (socket, mut poller) = match init_listener(evfd.as_fd()) {
        Ok(result) => result,
        Err(error) => {
            error!(?error, "failed to setup listener");
            queue.push(Err(error));
            return;
        }
    };
    let mut events = Events::with_capacity(opts.capacity);
    'main: loop {
        match poller.poll(&mut events, None) {
            Err(error) => {
                error!(?error, "failed to poll udev monitor");
                queue.push(Err(error));
                return;
            }
            Ok(_) => {
                for event in &events {
                    if event.token() == Token(0) && event.is_readable() {
                        trace!("closing listener");
                        let mut arr = [0; std::mem::size_of::<u64>()];
                        let _ = unistd::read(evfd.as_fd(), &mut arr);
                        queue.done();
                        break 'main;
                    } else if event.token() == Token(1) && event.is_read_closed() {
                        trace!("closing listener");
                        queue.done();
                        break 'main;
                    } else if event.token() == Token(1) && event.is_readable() {
                        for event in socket.iter() {
                            trace!(event = ?event.event_type(), "device event");
                            let dev = event.device();
                            let port = match dev.devnode() {
                                Some(path) => path.to_str().unwrap_or("").to_string(),
                                _ => "".to_string(),
                            };
                            let item = match event.event_type() {
                                udev::EventType::Add => Some(EventType::Add),
                                udev::EventType::Remove => Some(EventType::Remove),
                                _ => None,
                            };
                            if let Some(item) = item {
                                queue.push(Ok(EventInfo {
                                    device: read_device_info(port, &dev),
                                    event: item,
                                }));
                            }
                        }
                    }
                }
            }
        }
    }
    trace!("listener finished");
}

#[inline]
fn init_listener(evfd: BorrowedFd<'_>) -> io::Result<(udev::MonitorSocket, mio::Poll)> {
    let mut socket = udev::MonitorBuilder::new()?
        .match_subsystem("tty")?
        .listen()?;
    let poll = mio::Poll::new()?;
    poll.registry().register(
        &mut SourceFd(&evfd.as_raw_fd()),
        Token(0),
        Interest::READABLE,
    )?;
    poll.registry()
        .register(&mut socket, Token(1), Interest::READABLE)?;
    Ok((socket, poll))
}

fn read_device_info(port: String, dev: &Device) -> DeviceInfo {
    let serial = dev
        .property_value("ID_SERIAL_SHORT")
        .and_then(OsStr::to_str)
        .map(|s| s.to_string());
    let manufacturer = dev
        .property_value("ID_VENDOR_ENC")
        .and_then(OsStr::to_str)
        .and_then(|s| unescaper::unescape(s).ok().map(|s| s.to_string()))
        .or_else(|| {
            dev.property_value("ID_VENDOR")
                .and_then(OsStr::to_str)
                .map(|s| s.to_string().replace('_', " "))
        })
        .or_else(|| {
            dev.property_value("ID_VENDOR_FROM_DATABASE")
                .and_then(OsStr::to_str)
                .map(|s| s.to_string())
        });
    let product = dev
        .property_value("ID_MODEL_ENC")
        .and_then(OsStr::to_str)
        .and_then(|s| unescaper::unescape(s).ok().map(|s| s.to_string()))
        .or_else(|| {
            dev.property_value("ID_MODEL")
                .and_then(OsStr::to_str)
                .map(|s| s.to_string().replace('_', " "))
        })
        .or_else(|| {
            dev.property_value("ID_MODEL_FROM_DATABASE")
                .and_then(OsStr::to_str)
                .map(|s| s.to_string())
        });
    let vid = dev
        .property_value("ID_VENDOR_ID")
        .and_then(OsStr::to_str)
        .map(|s| s.to_string());
    let pid = dev
        .property_value("ID_MODEL_ID")
        .and_then(OsStr::to_str)
        .map(|s| s.to_string());
    DeviceInfo {
        port,
        serial,
        manufacturer,
        product,
        vid,
        pid,
    }
}

/// An event emitter to listen for Usb Add Remove events
pub struct EventIter {
    queue: Arc<Queue>,
}

impl Debug for EventIter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventIter").finish()
    }
}

impl Stream for EventIter {
    type Item = io::Result<EventInfo>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.queue.poll_next(cx)
    }
}

/// The AbortHandle will cause the [`EventIter`] to stop emitting events when dropped
#[derive(Debug)]
pub struct AbortHandle {
    evfd: EventFd,
    join_handle: Option<JoinHandle<()>>,
}

impl AbortHandle {
    /// Cancel [`EventIter`] and no longer listen to Device Connect and Disconnect events
    pub fn abort(self) {}
}

impl Drop for AbortHandle {
    // We signal the remote thread to break its loop with the eventfd, and then we join
    fn drop(&mut self) {
        trace!("dropping event iter");
        if let Some(jh) = self.join_handle.take() {
            match self.evfd.write(1) {
                Err(error) => error!(?error, "failed to write evfd"),
                Ok(_) => {
                    if let Err(error) = jh.join() {
                        error!(?error, "event iter join error");
                    }
                }
            }
        }
    }
}
