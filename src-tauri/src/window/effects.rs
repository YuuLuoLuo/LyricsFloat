//! 窗口视觉效果：磨砂玻璃 + 圆角裁剪 + 深色标题栏模式。
//!
//! 为什么不用 window-vibrancy：它调的 `ACCENT_ENABLE_ACRYLICBLURBEHIND` 在
//! Win11 22H2 之后由 DWM 接管，会跟随系统主题取色。系统是浅色主题时，
//! 亚克力会被画成一片偏白的灰 —— 这正是"磨砂变白底"的根因。
//!
//! 这里自己调 `SetWindowCompositionAttribute`，把深色调直接塞进
//! GradientColor，同时给窗口打上 `DWMWA_USE_IMMERSIVE_DARK_MODE`，
//! 让系统按深色主题渲染背板。
//!
//! 圆角由**窗口区域**（SetWindowRgn）裁出来，而不是 DWM 的
//! `DWMWA_WINDOW_CORNER_PREFERENCE`：
//! - DWM 画窗口级圆角时会给窗口配一层投影，静止态（全透明）时那层阴影
//!   浮在桌面上 ——「没有边框却有一圈暗影」就是它；
//! - 窗口一旦有了区域，DWM 就不再投影，也不再画框架（顺便消掉了
//!   偶尔冒出来的标题文字）；
//! - 亚克力背板跟着窗口区域一起被裁，四角不会出现对不齐的直角。

use tauri::WebviewWindow;

/// 圆角半径（逻辑像素）。对齐 Win11 默认的 8px 观感。
const CORNER_RADIUS: f64 = 8.0;

pub fn enable_glass(win: &WebviewWindow, tint: u32) {
    #[cfg(target_os = "windows")]
    {
        let Some(hwnd) = hwnd_of(win) else { return };
        imp::set_dark_mode(hwnd);
        imp::set_accent(hwnd, imp::ACCENT_ACRYLIC, tint);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (win, tint);
    }
}

pub fn disable_glass(win: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        if let Some(hwnd) = hwnd_of(win) {
            imp::set_accent(hwnd, imp::ACCENT_NONE, 0);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = win;
    }
}

/// WebView2 默认背景是白色。透明窗口里如果不显式把它设成全透明，
/// 整块窗口就是一片白 —— 这正是"磨砂变成白底"的元凶。
pub fn set_transparent_background(win: &WebviewWindow) {
    let _ = win.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
}

/// 窗口圆角偏好。交给 DWM 会附带投影，所以词板一律传 false，
/// 真正的圆角由 `apply_round_region` 用窗口区域裁出来。
pub fn set_system_corner(win: &WebviewWindow, round: bool) {
    #[cfg(target_os = "windows")]
    {
        if let Some(hwnd) = hwnd_of(win) {
            imp::set_corner_preference(hwnd, round);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (win, round);
    }
}

/// 按圆角矩形裁剪窗口（尺寸取窗口当前大小）。
pub fn apply_round_region(win: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        if let Some(hwnd) = hwnd_of(win) {
            imp::set_round_region(hwnd, CORNER_RADIUS);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = win;
    }
}

/// 去掉 DWM 的 1px 窗口边框（跟随强调色的细线）。
pub fn strip_window_border(win: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        if let Some(hwnd) = hwnd_of(win) {
            imp::strip_border(hwnd);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = win;
    }
}

#[cfg(target_os = "windows")]
fn hwnd_of(win: &WebviewWindow) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::HWND;
    let raw = win.hwnd().ok()?.0 as isize;
    if raw == 0 {
        return None;
    }
    Some(HWND(raw as *mut core::ffi::c_void))
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{BOOL, HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    pub const ACCENT_NONE: u32 = 0;
    pub const ACCENT_ACRYLIC: u32 = 4;

    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }

    #[repr(C)]
    struct CompositionAttribData {
        attrib: u32,
        data: *mut c_void,
        size: usize,
    }

    type SetWindowCompositionAttributeFn =
        unsafe extern "system" fn(HWND, *mut CompositionAttribData) -> BOOL;

    /// WCA_ACCENT_POLICY
    const WCA_ACCENT_POLICY: u32 = 19;

    /// DWMWA_USE_IMMERSIVE_DARK_MODE
    const DWMWA_USE_IMMERSIVE_DARK_MODE: i32 = 20;
    /// DWMWA_WINDOW_CORNER_PREFERENCE
    const DWMWA_WINDOW_CORNER_PREFERENCE: i32 = 33;
    /// DWMWA_BORDER_COLOR
    const DWMWA_BORDER_COLOR: i32 = 34;
    const DWMWCP_ROUND: i32 = 2;
    const DWMWCP_DONOTROUND: i32 = 1;
    /// DWMWA_COLOR_NONE：把这个值塞给 BORDER_COLOR 就是"不要画边框"
    const DWMWA_COLOR_NONE: i32 = 0xFFFF_FFFEu32 as i32;

    fn sca() -> Option<SetWindowCompositionAttributeFn> {
        static FN: OnceLock<Option<SetWindowCompositionAttributeFn>> = OnceLock::new();
        *FN.get_or_init(|| unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).ok()?;
            let name = windows::core::s!("SetWindowCompositionAttribute");
            let ptr = GetProcAddress(module, name)?;
            Some(std::mem::transmute(ptr))
        })
    }

    pub fn set_accent(hwnd: HWND, state: u32, gradient: u32) {
        let Some(f) = sca() else { return };
        let mut policy = AccentPolicy {
            state,
            flags: 2,
            gradient_color: gradient,
            animation_id: 0,
        };
        let mut data = CompositionAttribData {
            attrib: WCA_ACCENT_POLICY,
            data: &mut policy as *mut _ as *mut c_void,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        unsafe {
            let _ = f(hwnd, &mut data);
        }
    }

    fn dwm_set(hwnd: HWND, attr: i32, value: &i32) {
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(attr),
                value as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    pub fn set_dark_mode(hwnd: HWND) {
        let on: i32 = 1;
        dwm_set(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on);
    }

    pub fn set_corner_preference(hwnd: HWND, round: bool) {
        let v: i32 = if round { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
        dwm_set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &v);
    }

    /// Win11 会给每个顶层窗口描 1px 边框（跟随强调色的细线）。
    /// 词板静止时是 100% 透明的，这条线就成了"不悬停也能看见的边框"。
    pub fn strip_border(hwnd: HWND) {
        dwm_set(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
    }

    /// 把窗口裁成圆角矩形。
    ///
    /// 半径按真实 DPI 换算成物理像素：`win.scale_factor()` 在部分 DPI
    /// 组合下给 1.0，而 WebView2 实际按 1.5 渲染，直接乘它会裁歪，
    /// 所以这里问系统要 `GetDpiForWindow`。
    pub fn set_round_region(hwnd: HWND, radius_logical: f64) {
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_err() {
                return;
            }
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;
            if w <= 0 || h <= 0 {
                return;
            }
            let dpi = GetDpiForWindow(hwnd) as f64;
            let scale = if dpi > 0.0 { dpi / 96.0 } else { 1.0 };
            let r = (radius_logical * scale).round().max(1.0) as i32;
            // CreateRoundRectRgn 收的是椭圆宽高，也就是直径
            let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, r * 2, r * 2);
            if rgn.is_invalid() {
                return;
            }
            // SetWindowRgn 成功后区域归系统所有，不能再删
            if SetWindowRgn(hwnd, rgn, true) == 0 {
                let _ = DeleteObject(rgn);
            }
        }
    }
}
