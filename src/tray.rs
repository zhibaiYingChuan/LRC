// 许可证: Apache 2.0
//
// 系统托盘模块 — 桌面端后台运行支持
// =====================================
//
// 核心能力:
//   1. 创建系统托盘图标，显示 "LRC 记忆服务运行中" 提示
//   2. 右键菜单：打开仪表盘 / 退出
//   3. 双击托盘图标：打开浏览器仪表盘
//   4. 跨平台：Windows 原生 + Linux/macOS 降级提示
//
// 设计原则:
//   - Windows 用 Win32 Shell_NotifyIconW API（独立线程消息循环）
//   - Linux/macOS 打印提示而非阻塞
//   - 不影响 tokio 异步主循环

/// 启动系统托盘图标
///
/// Windows 上在独立线程中运行消息循环。
/// 非 Windows 平台打印提示后立即返回。
///
/// # 参数
/// - `dashboard_url`: 仪表盘地址
pub fn start_tray(dashboard_url: String) -> Result<TrayHandle, String> {
    #[cfg(windows)]
    {
        // 使用通道等待托盘线程初始化
        let (tx, rx) = std::sync::mpsc::channel();
        let url = dashboard_url.clone();

        std::thread::Builder::new()
            .name("lrc-tray".into())
            .spawn(move || {
                let _ = tx.send(());
                win_tray::run_tray_loop(&url);
            })
            .map_err(|e| format!("托盘线程启动失败: {e}"))?;

        // 等待线程就绪（最多1秒）
        let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
    }

    #[cfg(not(windows))]
    {
        eprintln!("[托盘] 系统托盘在非 Windows 平台暂不可用");
        eprintln!("[托盘] 仪表盘: {dashboard_url}");
        eprintln!("[托盘] Ctrl+C 退出服务");
    }

    Ok(TrayHandle {
        dashboard_url,
    })
}

/// 系统托盘句柄（仪表盘 URL 包装）
#[derive(Debug, Clone)]
pub struct TrayHandle {
    pub dashboard_url: String,
}

// ==================== Windows 实现 ====================

#[cfg(windows)]
mod win_tray {
    use std::ffi::OsStr;
    use std::mem;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
        NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage, RegisterClassW,
        SetForegroundWindow, TrackPopupMenu, TranslateMessage, CW_USEDEFAULT, HMENU, IDI_APPLICATION,
        MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_COMMAND, WM_CREATE,
        WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        GWLP_USERDATA, SetWindowLongPtrW, GetWindowLongPtrW,
    };

    const WM_TRAYICON: u32 = WM_USER + 1;
    const IDM_DASHBOARD: u32 = 1001;
    const IDM_EXIT: u32 = 1002;

    /// 启动托盘消息循环（在主线程中调用会阻塞）
    pub fn run_tray_loop(dashboard_url: &str) {
        // 转换为宽字符串
        let class_name: Vec<u16> = OsStr::new("LRC_TRAY_CLASS")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let window_name: Vec<u16> = OsStr::new("LRC Tray")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(tray_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) },
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };

        unsafe { RegisterClassW(&wc) };

        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                ptr::null_mut(),
            )
        };

        if hwnd != std::ptr::null_mut() {
            // 保存 dashboard_url 到窗口数据
            let url_ptr = Box::into_raw(Box::new(dashboard_url.to_string()));
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, url_ptr as isize) };

            // 添加托盘图标
            add_tray_icon(hwnd);

            // 消息循环
            let mut msg = unsafe { mem::zeroed::<MSG>() };
            loop {
                let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
                if ret == 0 || ret == -1 {
                    break;
                }
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // 清理
            unsafe {
                let _ = Box::from_raw(url_ptr);
            }
        }
    }

    fn add_tray_icon(hwnd: HWND) {
        let tooltip: Vec<u16> = OsStr::new("Loong Recall - AI 永久记忆系统")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };

        let copy_len = tooltip.len().min(127);
        nid.szTip[..copy_len].copy_from_slice(&tooltip[..copy_len]);

        unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    }

    fn remove_tray_icon(hwnd: HWND) {
        let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
    }

    /// 打开仪表盘
    fn open_dashboard(hwnd: HWND) {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr != 0 {
                let url = &*(ptr as *const String);
                if let Err(e) = webbrowser::open(url) {
                    eprintln!("[托盘] 打开浏览器失败: {e}");
                }
            }
        }
    }

    /// 显示右键菜单
    fn show_menu(hwnd: HWND) {
        unsafe {
            SetForegroundWindow(hwnd);

            let menu = CreatePopupMenu();
            if menu == std::ptr::null_mut() {
                return;
            }

            let dash_txt: Vec<u16> = OsStr::new("打开仪表盘")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            AppendMenuW(menu, MF_STRING, IDM_DASHBOARD as usize, dash_txt.as_ptr());

            let exit_txt: Vec<u16> = OsStr::new("退出")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            AppendMenuW(menu, MF_STRING, IDM_EXIT as usize, exit_txt.as_ptr());

            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);

            TrackPopupMenu(
                menu as HMENU,
                TPM_BOTTOMALIGN | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                0,
                hwnd,
                ptr::null(),
            );

            DestroyMenu(menu as HMENU);
        }
    }

    /// 窗口消息处理
    unsafe extern "system" fn tray_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CREATE => 0,
            WM_DESTROY => {
                remove_tray_icon(hwnd);
                PostQuitMessage(0);
                0
            }
            WM_TRAYICON => match lparam as u32 {
                WM_LBUTTONDBLCLK => {
                    open_dashboard(hwnd);
                    0
                }
                WM_RBUTTONUP => {
                    show_menu(hwnd);
                    0
                }
                _ => 0,
            },
            WM_COMMAND => match wparam as u32 {
                IDM_DASHBOARD => {
                    open_dashboard(hwnd);
                    0
                }
                IDM_EXIT => {
                    DestroyWindow(hwnd);
                    0
                }
                _ => DefWindowProcW(hwnd, msg, wparam, lparam),
            },
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}