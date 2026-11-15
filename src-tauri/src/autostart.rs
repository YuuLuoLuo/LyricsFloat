//! 开机自启：直接读写 HKCU 的 Run 键。
//!
//! 早先用 `reg.exe` 子进程实现，问题不少：每次查询要起一个进程（启动慢），
//! 而且在受限环境（企业策略、沙箱）里 `reg.exe` 可能被直接拦掉，
//! 会让整个自启判断变成永远 false。改成直接调注册表 API，零子进程、零依赖。

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE: &str = "LyricBoard";

#[cfg(windows)]
mod imp {
    use super::RUN_KEY;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
    };

    fn utf16(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(access: REG_SAM_FLAGS) -> Option<HKEY> {
        let mut key = HKEY::default();
        let sub = utf16(RUN_KEY);
        let rc =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), 0, access, &mut key) };
        if rc == ERROR_SUCCESS {
            Some(key)
        } else {
            None
        }
    }

    /// 值存在且内容非空才算启用
    pub fn query(name: &str) -> Option<String> {
        let key = open(KEY_QUERY_VALUE)?;
        let value_name = utf16(name);
        let mut size = 0u32;
        let rc = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                None,
                None,
                Some(&mut size),
            )
        };
        if rc != ERROR_SUCCESS || size <= 2 {
            let _ = unsafe { RegCloseKey(key) };
            return None;
        }

        let mut buf = vec![0u16; (size as usize / 2) + 1];
        let mut got = size;
        let rc = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(value_name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut got),
            )
        };
        let _ = unsafe { RegCloseKey(key) };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let len = (got as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
    }

    pub fn remove(name: &str) -> Result<(), String> {
        let Some(key) = open(KEY_SET_VALUE) else {
            return Ok(());
        };
        let value_name = utf16(name);
        let rc = unsafe { RegDeleteValueW(key, PCWSTR(value_name.as_ptr())) };
        let _ = unsafe { RegCloseKey(key) };
        // 值本来就不存在（ERROR_FILE_NOT_FOUND = 2）不算失败
        match rc.0 {
            0 | 2 => Ok(()),
            e => Err(format!("RegDeleteValue 失败: {e}")),
        }
    }

    pub fn write(name: &str, cmdline: &str) -> Result<(), String> {
        let Some(key) = open(KEY_SET_VALUE) else {
            return Err("无法打开 HKCU Run 键".into());
        };
        let value_name = utf16(name);
        // REG_SZ 的字节数必须含结尾 NUL
        let data: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
        let rc = unsafe {
            RegSetValueExW(key, PCWSTR(value_name.as_ptr()), 0, REG_SZ, Some(bytes))
        };
        let _ = unsafe { RegCloseKey(key) };
        if rc == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("RegSetValue 失败: {}", rc.0))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn query(_name: &str) -> Option<String> {
        None
    }
    pub fn remove(_name: &str) -> Result<(), String> {
        Ok(())
    }
    pub fn write(_name: &str, _cmdline: &str) -> Result<(), String> {
        Err("仅支持 Windows".into())
    }
}

pub fn is_enabled() -> bool {
    imp::query(VALUE).is_some()
}

pub fn apply(enabled: bool) -> Result<(), String> {
    if !enabled {
        return imp::remove(VALUE);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    imp::write(VALUE, &format!("\"{}\"", exe.display()))
}

#[cfg(windows)]
fn normalize(path: &str) -> String {
    path.trim().trim_matches('"').replace('/', "\\").to_lowercase()
}

/// 便携版会被挪位置，登记项里的路径可能失效 —— 启动时顺手校正一次。
pub fn sync_path() {
    let Some(registered) = imp::query(VALUE) else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let expected = format!("\"{}\"", exe.display());
    #[cfg(windows)]
    if normalize(&registered) == normalize(&expected) {
        return;
    }
    let _ = imp::write(VALUE, &expected);
}

#[tauri::command]
pub fn get_autostart() -> bool {
    is_enabled()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<bool, String> {
    apply(enabled).map(|_| true)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// 用独立值名做往返，绝不碰真实的 LyricBoard 项 ——
    /// 否则测试崩在中间会给用户留一条开机启动垃圾。
    const T: &str = "LyricBoardSelfTest";

    #[test]
    fn registry_roundtrip() {
        let _ = imp::remove(T);
        assert!(imp::query(T).is_none(), "初始状态应为空");

        imp::write(T, "\"C:\\tmp\\a b.exe\"").expect("写失败");
        assert_eq!(imp::query(T).as_deref(), Some("\"C:\\tmp\\a b.exe\""));

        // 覆盖写
        imp::write(T, "\"D:\\x\\y.exe\"").expect("覆盖写失败");
        assert_eq!(imp::query(T).as_deref(), Some("\"D:\\x\\y.exe\""));

        imp::remove(T).expect("删失败");
        assert!(imp::query(T).is_none(), "删除后应查不到");

        // 重复删不报错
        imp::remove(T).expect("重复删除应幂等");
    }

    #[test]
    fn path_normalization() {
        assert_eq!(normalize(r#""C:\A\b.EXE""#), normalize("c:/a/b.exe"));
    }
}
