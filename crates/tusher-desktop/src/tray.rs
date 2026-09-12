use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

pub struct TrayCallbacks {
    pub on_open_dashboard: Box<dyn Fn() + Send + Sync + 'static>,
    pub on_open_downloads: Box<dyn Fn() + Send + Sync + 'static>,
    pub on_toggle_sync: Box<dyn Fn() -> bool + Send + Sync + 'static>,
    pub on_toggle_context_menu: Box<dyn Fn() -> bool + Send + Sync + 'static>,
    pub on_exit: Box<dyn Fn() + Send + Sync + 'static>,
}

#[allow(dead_code)]
pub struct TrayHandle {
    stop_signal: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl TrayHandle {
    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
    }
}

#[cfg(windows)]
pub fn run_tray(callbacks: TrayCallbacks) -> Result<TrayHandle> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, RegisterClassW,
        SetForegroundWindow, TrackPopupMenu, TranslateMessage, IDI_APPLICATION, MF_DISABLED,
        MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_APP,
        WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW,
    };

    const WM_TRAY_CALLBACK: u32 = WM_APP + 100;
    const ID_HEADER: usize = 2001;
    const ID_OPEN_DASHBOARD: usize = 2002;
    const ID_OPEN_DOWNLOADS: usize = 2003;
    const ID_TOGGLE_SYNC: usize = 2004;
    const ID_CONTEXT_MENU: usize = 2005;
    const ID_EXIT: usize = 2006;

    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_signal_thread = Arc::clone(&stop_signal);

    std::thread::spawn(move || {
        unsafe {
            let class_name: Vec<u16> = std::ffi::OsStr::new("TusherTrayClass")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let h_instance = GetModuleHandleW(std::ptr::null());

            // Window Procedure Callback
            unsafe extern "system" fn wnd_proc(
                hwnd: HWND,
                msg: u32,
                wparam: WPARAM,
                lparam: LPARAM,
            ) -> LRESULT {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: h_instance,
                hIcon: LoadIconW(0, IDI_APPLICATION),
                hCursor: 0,
                hbrBackground: 0,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
            };

            RegisterClassW(&wc);

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                h_instance,
                std::ptr::null(),
            );

            if hwnd == 0 {
                warn!("Failed to create hidden window for tray");
                return;
            }

            let mut tip_chars = [0u16; 128];
            let tip_str = "T.U.S.H.E.R - Decentralized Sync Mesh";
            for (i, c) in tip_str.encode_utf16().enumerate() {
                if i < 127 {
                    tip_chars[i] = c;
                }
            }

            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                uCallbackMessage: WM_TRAY_CALLBACK,
                hIcon: LoadIconW(0, IDI_APPLICATION),
                szTip: tip_chars,
                dwState: 0,
                dwStateMask: 0,
                szInfo: [0; 256],
                Anonymous: std::mem::zeroed(),
                szInfoTitle: [0; 64],
                dwInfoFlags: 0,
                guidItem: std::mem::zeroed(),
                hBalloonIcon: 0,
            };

            Shell_NotifyIconW(NIM_ADD, &nid);
            info!("System tray icon registered successfully");

            let to_wide = |s: &str| -> Vec<u16> {
                std::ffi::OsStr::new(s)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect()
            };

            let mut msg: MSG = std::mem::zeroed();

            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                if stop_signal_thread.load(Ordering::SeqCst) {
                    break;
                }

                if msg.message == WM_TRAY_CALLBACK {
                    let event = msg.lParam as u32;
                    if event == WM_RBUTTONUP {
                        let mut pt: POINT = std::mem::zeroed();
                        GetCursorPos(&mut pt);
                        SetForegroundWindow(hwnd);

                        let hmenu = CreatePopupMenu();
                        
                        let h_header = to_wide("T.U.S.H.E.R (Active Mesh Node)");
                        let h_open = to_wide("🌐 Open Web Dashboard");
                        let h_down = to_wide("📂 Open Downloads Folder");
                        let h_sync = to_wide("⏸ Pause / Resume Sync");
                        let h_menu = to_wide("⚙ Explorer Context Menu (Toggle)");
                        let h_exit = to_wide("❌ Exit T.U.S.H.E.R");

                        AppendMenuW(hmenu, MF_STRING | MF_DISABLED | MF_GRAYED, ID_HEADER, h_header.as_ptr());
                        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
                        AppendMenuW(hmenu, MF_STRING, ID_OPEN_DASHBOARD, h_open.as_ptr());
                        AppendMenuW(hmenu, MF_STRING, ID_OPEN_DOWNLOADS, h_down.as_ptr());
                        AppendMenuW(hmenu, MF_STRING, ID_TOGGLE_SYNC, h_sync.as_ptr());
                        AppendMenuW(hmenu, MF_STRING, ID_CONTEXT_MENU, h_menu.as_ptr());
                        AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
                        AppendMenuW(hmenu, MF_STRING, ID_EXIT, h_exit.as_ptr());

                        let selected = TrackPopupMenu(
                            hmenu,
                            TPM_BOTTOMALIGN | TPM_LEFTALIGN | 0x0100, // TPM_RETURNCMD = 0x0100
                            pt.x,
                            pt.y,
                            0,
                            hwnd,
                            std::ptr::null(),
                        );

                        DestroyMenu(hmenu);

                        match selected as usize {
                            ID_OPEN_DASHBOARD => (callbacks.on_open_dashboard)(),
                            ID_OPEN_DOWNLOADS => (callbacks.on_open_downloads)(),
                            ID_TOGGLE_SYNC => {
                                let _ = (callbacks.on_toggle_sync)();
                            }
                            ID_CONTEXT_MENU => {
                                let _ = (callbacks.on_toggle_context_menu)();
                            }
                            ID_EXIT => {
                                (callbacks.on_exit)();
                                break;
                            }
                            _ => {}
                        }
                    } else if event == WM_LBUTTONDBLCLK {
                        (callbacks.on_open_dashboard)();
                    }
                }

                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            Shell_NotifyIconW(NIM_DELETE, &nid);
            info!("System tray icon deleted cleanly");
        }
    });

    Ok(TrayHandle { stop_signal })
}

#[cfg(not(windows))]
pub fn run_tray(callbacks: TrayCallbacks) -> Result<TrayHandle> {
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop_signal);

    std::thread::spawn(move || {
        info!("Tray running in headless/fallback mode on non-Windows OS");
        while !stop_thread.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });

    Ok(TrayHandle { stop_signal })
}
