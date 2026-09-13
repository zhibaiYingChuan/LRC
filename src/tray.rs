//! 许可证: Apache 2.0
//!
//! 系统托盘模块 — 桌面端后台运行支持
//! =====================================
//!
//! 核心能力:
//!   1. 创建系统托盘图标，显示 "LRC 记忆服务运行中" 提示
//!   2. 右键菜单：打开仪表盘 / 退出
//!   3. 双击托盘图标：打开浏览器仪表盘
//!   4. 跨平台：Windows 原生 + Linux/macOS 降级提示
//!
//! 设计原则:
//!   - Windows 用 Win32 Shell_NotifyIconW API（独立线程消息循环）
//!   - Linux/macOS 打印提示而非阻塞
//!   - 不影响 tokio 异步主循环

// v0.9.7（GLOBAL_CODE_REVIEW_REPORT P1-6）：托盘线程启动错误由不可判别的
// `String` 收敛为带域分类的 [`crate::errors::LrcError`]（内部错误）。
// `Display` 仅输出 message，故 CLI 日志文案**零漂移**。
use crate::errors::LrcResult;

/// 启动系统托盘图标
///
/// Windows 上在独立线程中运行消息循环。
/// 非 Windows 平台打印提示后立即返回。
///
/// # 参数
/// - `dashboard_url`: 仪表盘地址
pub fn start_tray(dashboard_url: String) -> LrcResult<TrayHandle> {
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
            .map_err(|e| crate::errors::LrcError::internal(format!("托盘线程启动失败: {e}")))?;

        // 等待线程就绪（最多1秒）
        let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
    }

    #[cfg(not(windows))]
    {
        eprintln!("[托盘] 系统托盘在非 Windows 平台暂不可用");
        eprintln!("[托盘] 仪表盘: {dashboard_url}");
        eprintln!("[托盘] Ctrl+C 退出服务");
    }

    Ok(TrayHandle { dashboard_url })
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
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, LoadIconW, PostQuitMessage,
        RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage,
        CW_USEDEFAULT, GWLP_USERDATA, HMENU, IDI_APPLICATION, MF_STRING, MSG, TPM_BOTTOMALIGN,
        TPM_LEFTALIGN, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_USER,
        WNDCLASSW, WS_OVERLAPPEDWINDOW,
    };

    const WM_TRAYICON: u32 = WM_USER + 1;
    const IDM_DASHBOARD: u32 = 1001;
    const IDM_EXIT: u32 = 1002;

    /// 窗口用户数据的 magic 标记
    ///
    /// v0.9.7 新增（配合「托盘裸指针无来源校验」修复）：
    ///   用于在解引用 GWLP_USERDATA 前证明该指针确由本模块写入，
    ///   避免把任意非 0 值当作有效 `String` 解引用（未定义行为）。
    const USER_DATA_MAGIC: u64 = 0x4C52_4354_5241_5901; // "LRCTRAY" + 版本位

    /// 存入窗口用户数据的包裹结构（带 magic tag，供来源校验）
    struct UserData {
        magic: u64,
        url: String,
    }

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

        // SAFETY: GetModuleHandleW(nullptr) 返回当前模块句柄，始终有效，无内存操作
        let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(tray_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            // SAFETY: LoadIconW(nullptr, IDI_APPLICATION) 加载系统默认图标，两个参数均为常量
            hIcon: unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) },
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };

        // SAFETY: RegisterClassW 注册窗口类，wc 为栈上有效结构体，所有字段已正确初始化
        unsafe { RegisterClassW(&wc) };

        // SAFETY: CreateWindowExW 创建消息窗口，所有参数均为有效值或空指针
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

        if !hwnd.is_null() {
            // 保存 dashboard_url 到窗口数据
            //
            // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P3 安全「托盘裸指针无来源校验」）：
            //   根因：原实现把 `Box::into_raw(Box::new(String))` 的裸指针直接存入 GWLP_USERDATA，
            //         读取侧仅做 `ptr != 0` 检查就 `&*(ptr as *const String)` 解引用——
            //         无任何来源/类型校验。若该窗口数据被其它消息（如 WM_GETMINMAXINFO 等
            //         早于本处赋值的路径）写入非 0 的其它值，或消息循环退出后仍有派发
            //         （use-after-free 窗口期），解引用即为未定义行为。
            //   修复：改用带 **magic tag** 的包裹结构 `UserData { magic, url }`；
            //         读取侧先校验 magic 再取字段，且用 `addr_of!` 取字段地址避免
            //         对可能失效的整体引用求值，消除"把任意非 0 值当 String"的风险。
            let user_data_ptr = Box::into_raw(Box::new(UserData {
                magic: USER_DATA_MAGIC,
                url: dashboard_url.to_string(),
            }));
            // SAFETY: SetWindowLongPtrW 设置窗口用户数据；user_data_ptr 由 Box::into_raw 分配，
            //         生命周期由本函数末尾的 Box::from_raw 回收（窗口销毁前完成）。
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, user_data_ptr as isize) };

            // 添加托盘图标
            add_tray_icon(hwnd);

            // 消息循环
            // SAFETY: mem::zeroed 初始化 MSG 结构体，MSG 是 POD 类型，零初始化是安全的
            let mut msg = unsafe { mem::zeroed::<MSG>() };
            loop {
                // SAFETY: GetMessageW 从消息队列获取消息，msg 是栈上有效变量
                let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
                if ret == 0 || ret == -1 {
                    break;
                }
                // SAFETY: TranslateMessage 和 DispatchMessageW 是标准消息处理函数，msg 已由 GetMessageW 填充
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }

            // 清理
            // SAFETY: Box::from_raw 从 SetWindowLongPtrW 保存的指针重建 Box。
            //         与写入侧配对，且此处置空窗口数据，消除消息循环退出后的 use-after-free 窗口。
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let _ = Box::from_raw(user_data_ptr);
            }
        }
    }

    fn add_tray_icon(hwnd: HWND) {
        let tooltip: Vec<u16> = OsStr::new("Loong Recall - AI 永久记忆系统")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: mem::zeroed 初始化 NOTIFYICONDATAW 结构体，该结构体是 POD 类型
        let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        // SAFETY: LoadIconW(nullptr, IDI_APPLICATION) 加载系统默认图标
        nid.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };

        let copy_len = tooltip.len().min(127);
        nid.szTip[..copy_len].copy_from_slice(&tooltip[..copy_len]);

        // SAFETY: Shell_NotifyIconW(NIM_ADD) 添加托盘图标，nid 已正确初始化
        unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    }

    fn remove_tray_icon(hwnd: HWND) {
        // SAFETY: mem::zeroed 初始化 NOTIFYICONDATAW 结构体
        let mut nid: NOTIFYICONDATAW = unsafe { mem::zeroed() };
        nid.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        // SAFETY: Shell_NotifyIconW(NIM_DELETE) 删除托盘图标，nid 已正确初始化
        unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
    }

    /// 打开仪表盘
    fn open_dashboard(hwnd: HWND) {
        // v0.9.7 修复（GLOBAL_CODE_REVIEW_REPORT P3 安全「托盘裸指针无来源校验」）：
        //   原实现仅检查 `ptr != 0` 就把窗口数据当 `*const String` 解引用，无类型/来源校验。
        //   现要求：(1) 指针非 0；(2) magic 匹配 `USER_DATA_MAGIC`（证明确由本模块写入）；
        //   否则直接返回，不再解引用任意非 0 值。
        // SAFETY: GetWindowLongPtrW 读取先前由 SetWindowLongPtrW 写入的窗口用户数据指针；
        //         解引用前已双重校验（非空 + magic 匹配），且仅在消息循环存活期内被调用
        //         （消息循环退出时会先置空该数据，故不存在 use-after-free）。
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if ptr == 0 {
                return;
            }
            let user_data = &*(ptr as *const UserData);
            if user_data.magic != USER_DATA_MAGIC {
                eprintln!(
                    "[托盘] 窗口用户数据 magic 校验失败，跳过打开仪表盘（防止非法指针解引用）"
                );
                return;
            }
            if let Err(e) = webbrowser::open(&user_data.url) {
                eprintln!("[托盘] 打开浏览器失败: {e}");
            }
        }
    }

    /// 显示右键菜单
    fn show_menu(hwnd: HWND) {
        unsafe {
            SetForegroundWindow(hwnd);

            let menu = CreatePopupMenu();
            if menu.is_null() {
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
