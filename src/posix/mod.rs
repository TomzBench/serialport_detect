// Posix support

use crate::detect::{DeviceInfo, EventInfo, EventType, Queue};
use futures::Stream;
use mio::{Events, Interest, Token};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt::{self, Debug},
    io,
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
}

pub(crate) fn scan() -> io::Result<HashMap<String, EventInfo>> {
    let mut enumerator = udev::Enumerator::new()?;
    enumerator.match_subsystem("tty")?;
    let items = enumerator
        .scan_devices()?
        .into_iter()
        .map(|dev| {
            let port = match dev.devnode() {
                Some(path) => path.to_str().unwrap_or("").to_string(),
                _ => "".to_string(),
            };
            let info = EventInfo {
                meta: read_device_info(&dev),
                port: port.clone(),
                event: EventType::Add,
            };
            (port, info)
        })
        .collect();
    Ok(items)
}

/// Listen for connected devices
pub fn listen() -> EventIter {
    let queue = Arc::new(Queue::new());
    let theirs = Arc::clone(&queue);
    let opts = ListenerOptions { capacity: 1024 };
    let jh = std::thread::spawn(move || listener(theirs, opts));
    EventIter {
        queue,
        join_handle: Some(jh),
    }
}

fn listener(queue: Arc<Queue>, opts: ListenerOptions) {
    // Get a udev socket
    trace!(capacity = opts.capacity, "listening");
    let (socket, mut poller) = match init_listener() {
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
                    trace!(event=?event, "mio event");
                    if event.token() == Token(0) && event.is_read_closed() {
                        break 'main;
                    } else if event.token() == Token(0) && event.is_readable() {
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
                                    meta: read_device_info(&dev),
                                    port,
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

fn read_device_info(dev: &Device) -> DeviceInfo {
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
        serial,
        manufacturer,
        product,
        vid,
        pid,
    }
}

#[inline]
fn init_listener() -> io::Result<(udev::MonitorSocket, mio::Poll)> {
    let mut socket = udev::MonitorBuilder::new()?
        .match_subsystem("tty")?
        .listen()?;
    let poll = mio::Poll::new()?;
    poll.registry()
        .register(&mut socket, Token(0), Interest::READABLE)?;
    Ok((socket, poll))
}

/// An event emitter to listen for Usb Add Remove events
pub struct EventIter {
    queue: Arc<Queue>,
    join_handle: Option<JoinHandle<()>>,
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

impl Drop for EventIter {
    fn drop(&mut self) {
        if let Some(jh) = self.join_handle.take() {
            self.queue.done();
            if let Err(error) = jh.join() {
                trace!(?error, "event iter join error");
            }
        }
    }
}
