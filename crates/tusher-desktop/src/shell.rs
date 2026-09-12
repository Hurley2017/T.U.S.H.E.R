use anyhow::Result;
use std::path::Path;
use tracing::info;

#[cfg(windows)]
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW,
    HKEY_CURRENT_USER, KEY_READ, REG_SZ,
};

/// Install the "Share via T.U.S.H.E.R" context menu in Windows Explorer.
/// Installs under HKCU\Software\Classes\*\shell\TusherShare and
/// HKCU\Software\Classes\Directory\shell\TusherShare (no admin rights needed).
pub fn install_context_menu(exe_path: Option<&Path>) -> Result<()> {
    #[cfg(windows)]
    {
        let current_exe = match exe_path {
            Some(p) => p.to_path_buf(),
            None => std::env::current_exe()?,
        };
        let exe_str = current_exe.to_string_lossy().to_string();

        let entries = [
            "Software\\Classes\\*\\shell\\TusherShare",
            "Software\\Classes\\Directory\\shell\\TusherShare",
        ];

        for entry in entries {
            let menu_label = "Share via T.U.S.H.E.R\0";
            let icon_val = format!("\"{}\",0\0", exe_str);
            let cmd_val = format!("\"{}\" share \"%1\"\0", exe_str);

            set_registry_string(entry, "", menu_label)?;
            set_registry_string(entry, "Icon\0", &icon_val)?;
            
            let cmd_key = format!("{}\\command", entry);
            set_registry_string(&cmd_key, "", &cmd_val)?;
        }

        info!("Successfully installed Windows Explorer context menu for T.U.S.H.E.R");
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = exe_path;
        info!("Context menu registration is only supported on Windows currently");
        Ok(())
    }
}

/// Uninstall the Windows Explorer context menu.
pub fn uninstall_context_menu() -> Result<()> {
    #[cfg(windows)]
    {
        let entries = [
            "Software\\Classes\\*\\shell\\TusherShare",
            "Software\\Classes\\Directory\\shell\\TusherShare",
        ];

        for entry in entries {
            delete_registry_tree(entry)?;
        }

        info!("Successfully uninstalled Windows Explorer context menu for T.U.S.H.E.R");
        Ok(())
    }

    #[cfg(not(windows))]
    {
        info!("Context menu uninstallation is only supported on Windows currently");
        Ok(())
    }
}

/// Check if the Windows Explorer context menu is currently installed.
pub fn is_context_menu_installed() -> bool {
    #[cfg(windows)]
    {
        let subkey = to_wide_chars("Software\\Classes\\*\\shell\\TusherShare");
        let mut hkey: windows_sys::Win32::System::Registry::HKEY = 0;
        unsafe {
            let status = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_READ,
                &mut hkey,
            );
            if status == 0 && hkey != 0 {
                RegCloseKey(hkey);
                true
            } else {
                false
            }
        }
    }

    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn set_registry_string(subkey_path: &str, value_name: &str, data: &str) -> Result<()> {
    let subkey_wide = to_wide_chars(subkey_path);
    let mut hkey: windows_sys::Win32::System::Registry::HKEY = 0;

    unsafe {
        let status = RegCreateKeyW(
            HKEY_CURRENT_USER,
            subkey_wide.as_ptr(),
            &mut hkey,
        );
        if status != 0 || hkey == 0 {
            anyhow::bail!("Failed to create/open registry key: {} (error code {})", subkey_path, status);
        }

        let name_wide: Vec<u16> = if value_name.is_empty() {
            vec![0]
        } else {
            to_wide_chars(value_name.trim_end_matches('\0'))
        };

        let data_wide = to_wide_chars(data.trim_end_matches('\0'));
        let byte_len = (data_wide.len() * std::mem::size_of::<u16>()) as u32;

        let set_status = RegSetValueExW(
            hkey,
            if value_name.is_empty() { std::ptr::null() } else { name_wide.as_ptr() },
            0,
            REG_SZ,
            data_wide.as_ptr() as *const u8,
            byte_len,
        );

        RegCloseKey(hkey);

        if set_status != 0 {
            anyhow::bail!("Failed to set registry value in {}: (error code {})", subkey_path, set_status);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn delete_registry_tree(subkey_path: &str) -> Result<()> {
    let subkey_wide = to_wide_chars(subkey_path);
    unsafe {
        let status = RegDeleteTreeW(HKEY_CURRENT_USER, subkey_wide.as_ptr());
        if status != 0 && status != 2 {
            // 2 is ERROR_FILE_NOT_FOUND (already deleted)
            anyhow::bail!("Failed to delete registry tree: {} (error code {})", subkey_path, status);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn to_wide_chars(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
