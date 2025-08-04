mod guid;
mod wide;
mod wm;

use crate::{
    detect::{DeviceInfo, Queue},
    EventInfo,
};
use futures::Stream;
use parking_lot::Mutex;
use serialport::SerialPortType;
use std::{
    collections::HashMap,
    ffi::OsString,
    fmt::{self, Debug},
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{error, trace};
use wide::to_wide;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE};

/// The AbortHandle will cause the [`EventIter`] to stop emitting events when dropped
#[derive(Debug)]
pub struct AbortHandle {
    window: OsString,
    join_handle: Option<JoinHandle<io::Result<()>>>,
}

impl AbortHandle {
    /// Cancel [`EventIter`] and no longer listen to Device Connect and Disconnect events
    pub fn abort(self) {}
}

impl Drop for AbortHandle {
    fn drop(&mut self) {
        let wide = to_wide(&self.window);
        let hwnd = unsafe {
            let result = FindWindowW(wm::WINDOW_CLASS_NAME, wide.as_ptr());
            match result.is_null() {
                false => result,
                _ => {
                    error!(error = ?io::Error::last_os_error(), "failed to abort");
                    return;
                }
            }
        };
        match unsafe { PostMessageW(hwnd as _, WM_CLOSE, 0, 0) } {
            0 => error!(error = ?io::Error::last_os_error()),
            _ => match self.join_handle.take() {
                None => unreachable!(),
                Some(jh) => match jh.join() {
                    Ok(_) => trace!("device detection closed"),
                    Err(error) => error!(?error, "device detection close error"),
                },
            },
        }
    }
}

pub(crate) struct IterState {
    pub(crate) cache: Mutex<HashMap<String, DeviceInfo>>,
    pub(crate) queue: Queue,
}

/// An event emitter to listen for Usb Add Remove events
pub struct EventIter {
    pub(crate) state: Arc<IterState>,
}

impl Debug for EventIter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventIter").finish()
    }
}

impl Stream for EventIter {
    type Item = io::Result<EventInfo>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.state.queue.poll_next(cx)
    }
}

pub(crate) fn listen() -> io::Result<(AbortHandle, EventIter)> {
    // We generate a random window name for our window manager device port listener
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|dur| dur.subsec_nanos())
        .unwrap_or(18825437)
        .to_string();
    let window = OsString::from(format!("SERIALPORT_DETECT{nanos}"));
    let name = window.clone();

    // Create polling context
    let state = Arc::new(IterState {
        cache: Mutex::new(scan()?),
        queue: Queue::new(),
    });
    let theirs = Arc::clone(&state);
    let jh = std::thread::spawn(move || unsafe {
        wm::window_dispatcher(name, Arc::into_raw(theirs) as _)
    });

    // Return an abort handle and a stream
    let abort_handle = AbortHandle {
        window,
        join_handle: Some(jh),
    };
    Ok((abort_handle, EventIter { state }))
}

pub(crate) fn scan() -> io::Result<HashMap<String, DeviceInfo>> {
    let devices = serialport::available_ports()?
        .into_iter()
        .filter_map(|info| match info.port_type {
            SerialPortType::UsbPort(usb) => {
                let port = info.port_name;
                let info = DeviceInfo {
                    port: port.clone(),
                    vid: Some(format!("{:X}", usb.vid)),
                    pid: Some(format!("{:X}", usb.pid)),
                    serial: usb.serial_number,
                    manufacturer: usb.manufacturer,
                    product: usb.product,
                };
                Some((port, info))
            }
            _ => None,
        })
        .collect::<HashMap<String, _>>();
    Ok(devices)
}
