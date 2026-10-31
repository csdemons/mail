//! Windows: явные права на файлы с данными программы — только текущий пользователь,
//! SYSTEM и администраторы (защищённый DACL без наследования от родителя). В Linux
//! то же обеспечивают права 0600/0700 при создании; там эти функции ничего не делают.

use std::path::Path;

/// Закрыть файл или каталог от других пользователей. Ошибка не мешает работе
/// (права профиля пользователя и так закрывают его каталоги) — пишется в журнал.
pub fn restrict(path: &Path) {
    #[cfg(windows)]
    if let Err(e) = win::restrict(path) {
        log::warn!("ACL for {} not set: {e}", path.display());
    }
    #[cfg(not(windows))]
    let _ = path;
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::{GetTokenInformation, SetFileSecurityW, TokenUser, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// SID текущего пользователя строкой (S-1-5-21-…).
    fn user_sid() -> Result<String, String> {
        // SAFETY: дескриптор маркера закрывается; буфер TOKEN_USER нужного размера,
        // строка SID освобождается LocalFree.
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err("OpenProcessToken".into());
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return Err("GetTokenInformation".into());
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut s: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
                return Err("ConvertSidToStringSidW".into());
            }
            let n = (0..).take_while(|&i| *s.add(i) != 0).count();
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
            LocalFree(s.cast());
            Ok(sid)
        }
    }

    pub fn restrict(path: &Path) -> Result<(), String> {
        let sid = user_sid()?;
        let inherit = if path.is_dir() { "OICI" } else { "" };
        let sddl = format!("D:P(A;{inherit};FA;;;{sid})(A;{inherit};FA;;;SY)(A;{inherit};FA;;;BA)");
        let sddl_w = wide(std::ffi::OsStr::new(&sddl));
        let path_w = wide(path.as_os_str());
        // SAFETY: строки нуль-терминированы; дескриптор безопасности освобождается LocalFree.
        unsafe {
            let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl_w.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) == 0 {
                return Err("ConvertStringSecurityDescriptorToSecurityDescriptorW".into());
            }
            let ok = SetFileSecurityW(path_w.as_ptr(), DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, sd);
            LocalFree(sd.cast());
            if ok == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
}
