use crate::{
    detect::{EventInfo, EventType},
    guid,
    windows::{wide::*, IterState},
};
use std::{
    ffi::{c_void, OsString},
    io,
    sync::Arc,
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{GetLastError, SetLastError, HMODULE, HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

/// The name of our window class.
/// [See also](https://learn.microsoft.com/en-us/windows/win32/winmsg/about-window-classes)
pub(crate) const WINDOW_CLASS_NAME: *const u16 = windows_sys::w!("DeviceNotifier");

/// Create an instance of a DeviceNotifier window.
///
/// Safety: name must be a null terminated Wide string, and user_data must be a pointer to an
unsafe fn create_window(name: *const u16, user_data: isize) -> io::Result<HWND> {
    let handle = CreateWindowExW(
        WS_EX_APPWINDOW,      // styleEx
        WINDOW_CLASS_NAME,    // class name
        name,                 // window name
        WS_MINIMIZE,          // style
        0,                    // x
        0,                    // y
        CW_USEDEFAULT,        // width
        CW_USEDEFAULT,        // hight
        std::ptr::null_mut(), // parent
        std::ptr::null_mut(), // menu
        hinstance(),          // instance
        std::ptr::null(),     // data
    );
    match handle.is_null() {
        true => Err(io::Error::last_os_error()),
        false => {
            // NOTE a 0 is returned if their is a failure, or if the previous pointer was NULL. To
            // distinguish if a true error has occured we have to clear any errors and test the
            // last_os_error == 0 or not.
            let prev = unsafe {
                SetLastError(0);
                SetWindowLongPtrW(handle, GWLP_USERDATA, user_data)
            };
            match prev {
                0 => match unsafe { GetLastError() } as _ {
                    0 => Ok(handle),
                    raw => Err(io::Error::from_raw_os_error(raw)),
                },
                _ => Ok(handle),
            }
        }
    }
}

/// Window proceedure for responding to windows messages and listening for device notifications
unsafe extern "system" fn window_proceedure(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const IterState;
    if !ptr.is_null() {
        let state = &*ptr;
        match msg {
            WM_DEVICECHANGE => {
                match parse_event(wparam) {
                    Some(EventType::Add) => {
                        if let Some(event) = crate::scan().ok().and_then(|scan| {
                            scan.into_iter().find_map(|(port, device)| {
                                // Safety: data is a DEV_BROADCAST_HDR when wparam is DBT_DEVICEARRIVAL
                                match unsafe { maybe_serialport(lparam as _) }? == port {
                                    false => None,
                                    true => Some(EventInfo {
                                        device,
                                        event: EventType::Add,
                                    }),
                                }
                            })
                        }) {
                            state
                                .cache
                                .lock()
                                .insert(event.device.port.clone(), event.device.clone());
                            state.queue.push(Ok(event));
                        }
                        0
                    }
                    Some(EventType::Remove) => {
                        // Safety: data is a DEV_BROADCAST_HDR when wparam is DBT_DEVICEARRIVAL
                        if let Some(event) = unsafe { maybe_serialport(lparam as _) }
                            .and_then(|want| state.cache.lock().remove(&want))
                            .map(|device| EventInfo {
                                device,
                                event: EventType::Remove,
                            })
                        {
                            state.queue.push(Ok(event))
                        };
                        0
                    }
                    None => {
                        // Just ignore the event
                        0
                    }
                }
            }
            WM_DESTROY => {
                // NOTE we only reconstruct our arc on destroy
                let arc = Arc::from_raw(ptr);
                arc.queue.done();
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn parse_event(wparam: WPARAM) -> Option<EventType> {
    match wparam as u32 {
        DBT_DEVICEARRIVAL => Some(EventType::Add),
        DBT_DEVICEREMOVECOMPLETE => Some(EventType::Remove),
        _ => None,
    }
}

/// Safety: data must be a DEV_BROADCAST_HDR
unsafe fn maybe_serialport(data: *mut c_void) -> Option<String> {
    let broadcast = &mut *(data as *mut DEV_BROADCAST_HDR);
    match broadcast.dbch_devicetype {
        DBT_DEVTYP_PORT => {
            let data = &*(data as *const DEV_BROADCAST_PORT_W);
            from_wide(data.dbcp_name.as_ptr())
                .to_str()
                .map(|port| port.to_string())
        }
        _ => None,
    }
}

/// Dispatch window messages
///
/// We receive a "name", a list of GUID registrations, and some "user_data" which is an arc.
///
/// Safety: user_data must outlive window procedure
///
/// This method will rebuild the Arc and pass it to the window procedure...
pub unsafe fn window_dispatcher(name: OsString, user_data: isize) -> io::Result<()> {
    const WCEUSBS: GUID =
        guid!(0x25dbce51, 0x6c8f, 0x4a72, 0x8a, 0x6d, 0xb5, 0x4c, 0x2b, 0x4f, 0xc8, 0x35);
    const USBDEVICE: GUID =
        guid!(0x88BAE032, 0x5A81, 0x49f0, 0xBC, 0x3D, 0xA4, 0xFF, 0x13, 0x82, 0x16, 0xD6);
    const PORTS: GUID =
        guid!(0x4d36e978, 0xe325, 0x11ce, 0xbf, 0xc1, 0x08, 0x00, 0x2b, 0xe1, 0x03, 0x18);
    let class = WNDCLASSEXW {
        style: 0,
        hIcon: std::ptr::null_mut(),
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as _,
        hIconSm: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: WINDOW_CLASS_NAME,
        lpfnWndProc: Some(window_proceedure),
        hbrBackground: std::ptr::null_mut(),
    };
    let _atom = match unsafe { RegisterClassExW(&class as *const _) } {
        0 => panic!("{:?}", io::Error::last_os_error()),
        atom => atom,
    };

    let unsafe_name = to_wide(name.clone());
    let arc = Arc::from_raw(user_data as *const Arc<IterState>);
    let hwnd = create_window(unsafe_name.as_ptr(), Arc::as_ptr(&arc) as _)?;
    let _registery = [WCEUSBS, USBDEVICE, PORTS]
        .into_iter()
        .map(|guid| {
            let handle = unsafe {
                let mut iface = std::mem::zeroed::<DEV_BROADCAST_DEVICEINTERFACE_W>();
                iface.dbcc_size = std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as _;
                iface.dbcc_classguid = guid;
                iface.dbcc_devicetype = DBT_DEVTYP_DEVICEINTERFACE;
                RegisterDeviceNotificationW(
                    hwnd as _,
                    &iface as *const _ as _,
                    DEVICE_NOTIFY_WINDOW_HANDLE,
                )
            };
            match handle.is_null() {
                false => Ok(handle),
                true => Err(io::Error::last_os_error()),
            }
        })
        .collect::<io::Result<Vec<_>>>()?;

    let mut msg: MSG = std::mem::zeroed();
    loop {
        match GetMessageW(&mut msg as *mut _, std::ptr::null_mut(), 0, 0) {
            0 => {
                break Ok(());
            }
            -1 => {
                let error = Err(io::Error::last_os_error());
                break error;
            }
            _ if msg.message == WM_CLOSE => {
                TranslateMessage(&msg as *const _);
                DispatchMessageW(&msg as *const _);
                break Ok(());
            }
            _ => {
                TranslateMessage(&msg as *const _);
                DispatchMessageW(&msg as *const _);
            }
        }
    }
}

/// Creating Windows requires the hinstance prop of the WinMain function. To retreive this
/// parameter use [`windows_sys::Win32::System::LibraryLoader::GetModuleHandleW`];
fn hinstance() -> HMODULE {
    // Safety: If the handle is NULL, GetModuleHandle returns a handle to the file used to create
    // the calling process
    unsafe { GetModuleHandleW(std::ptr::null()) }
}
