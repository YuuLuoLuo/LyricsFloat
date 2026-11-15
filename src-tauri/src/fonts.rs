//! 枚举系统已安装字体（DirectWrite）。
//!
//! 前端不能直接问浏览器"装了哪些字体"：`document.fonts` 只报已加载的 web font，
//! Local Font Access API 在 WebView2 里要么没有、要么弹权限框。
//! DirectWrite 的系统字体集合是唯一可靠来源，所以放在原生侧做。

/// 返回去重、排好序的字体家族名。
///
/// 优先取中文名（"微软雅黑"），没有中文名再退回英文名 ——
/// 用户在自己的系统里认得的是中文名。
#[tauri::command]
pub fn list_fonts() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        imp::families()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::collections::BTreeSet;

    use windows::core::PCWSTR;
    use windows::Win32::Graphics::DirectWrite::{
        DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, DWRITE_FACTORY_TYPE_SHARED,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 从一组本地化名字里挑一个可读的：zh-cn -> zh-tw -> en-us -> 第一个
    unsafe fn pick_name(
        names: &windows::Win32::Graphics::DirectWrite::IDWriteLocalizedStrings,
    ) -> Option<String> {
        let mut idx: u32 = 0;
        let mut exists = windows::Win32::Foundation::BOOL(0);

        for lang in ["zh-cn", "zh-hans", "en-us", "en"] {
            let l = wide(lang);
            idx = 0;
            exists = windows::Win32::Foundation::BOOL(0);
            let _ = names.FindLocaleName(PCWSTR(l.as_ptr()), &mut idx, &mut exists);
            if exists.as_bool() {
                break;
            }
        }
        let index = if exists.as_bool() { idx } else { 0 };

        let len = names.GetStringLength(index).ok()?;
        let mut buf = vec![0u16; len as usize + 1];
        names.GetString(index, &mut buf).ok()?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let text = String::from_utf16_lossy(&buf[..end]).trim().to_string();
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub fn families() -> Vec<String> {
        unsafe {
            let factory: IDWriteFactory = match DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) {
                Ok(f) => f,
                Err(_) => return Vec::new(),
            };
            let mut coll: Option<IDWriteFontCollection> = None;
            if factory.GetSystemFontCollection(&mut coll, false).is_err() {
                return Vec::new();
            }
            let Some(coll) = coll else { return Vec::new() };

            let count = coll.GetFontFamilyCount();
            // BTreeSet 顺便完成去重与排序（中文字体按拼音排在这里不理想，
            // 前端会再按"中文优先"分组，这里只要稳定即可）
            let mut out: BTreeSet<String> = BTreeSet::new();
            for i in 0..count {
                let Ok(family) = coll.GetFontFamily(i) else {
                    continue;
                };
                let Ok(names) = family.GetFamilyNames() else {
                    continue;
                };
                if let Some(name) = pick_name(&names) {
                    // "@SimSun" 这类是竖排专用的别名，选它反而是坑
                    if name.starts_with('@') {
                        continue;
                    }
                    out.insert(name);
                }
            }
            out.into_iter().collect()
        }
    }
}
