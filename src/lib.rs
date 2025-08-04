//! serialport-async is a small library of cross platform async helpers for extending serialport-rs
//!
//! # Feature Overview
//!
//! We provide a way to be notified when a new USB device has been added or removed from the
//! system.
//!
//! We spawn a thread for each open device to provide an async non blocking API to communicate with
//! the serialport device. This is considered appropriate because the number of serial ports
//! connected to a system is considered small. If you prefer a pure async approach, see mio-serial
//! and tokio-serial crates.

#![deny(
    clippy::dbg_macro,
    //missing_docs,
    missing_debug_implementations,
    missing_copy_implementations
)]
// Document feature-gated elements on docs.rs. See
// https://doc.rust-lang.org/rustdoc/unstable-features.html?highlight=doc(cfg#doccfg-recording-what-platforms-or-features-are-required-for-code-to-be-present
// and
// https://doc.rust-lang.org/rustdoc/unstable-features.html#doc_auto_cfg-automatically-generate-doccfg
// for details.
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
// Don't worry about needing to `unwrap()` or otherwise handle some results in
// doc tests.
#![doc(test(attr(allow(unused_must_use))))]

mod detect;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{AbortHandle, EventIter};

#[cfg(unix)]
mod posix;
use std::collections::HashMap;

#[cfg(unix)]
pub use posix::{AbortHandle, EventIter};

pub use detect::{DeviceInfo, EventInfo, EventType};

/// Listen for events
pub fn listen() -> std::io::Result<(AbortHandle, EventIter)> {
    #[cfg(unix)]
    return posix::listen();
    #[cfg(windows)]
    return windows::listen();
}

pub fn scan() -> std::io::Result<HashMap<String, DeviceInfo>> {
    #[cfg(unix)]
    return posix::scan();
    #[cfg(windows)]
    return windows::scan();
}
