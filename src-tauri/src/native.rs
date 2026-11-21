use std::{ffi::OsStr, path::Path, process::Command};

#[cfg(windows)]
fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

pub fn open(value: &OsStr) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                wide("open").as_ptr(),
                wide(value).as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!(
                "Windows 无法打开此项目（代码 {}）",
                result as isize
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(value)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

pub fn copy_text(value: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::GlobalFree,
            System::{
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
                Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            },
            UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE},
        };
        let text = wide(value);
        // Windows owns the moveable allocation after SetClipboardData succeeds.
        unsafe {
            let owner = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("ToolboxClipboard").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if owner.is_null() {
                return Err("无法建立剪贴板所有者。".into());
            }
            if OpenClipboard(owner) == 0 {
                DestroyWindow(owner);
                return Err("剪贴板暂时被其他应用占用，请重试。".into());
            }
            let result = (|| {
                let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2);
                if memory.is_null() {
                    return Err("无法分配剪贴板内存".into());
                }
                let pointer = GlobalLock(memory);
                if pointer.is_null() {
                    GlobalFree(memory);
                    return Err("无法写入剪贴板".into());
                }
                std::ptr::copy_nonoverlapping(text.as_ptr(), pointer.cast::<u16>(), text.len());
                GlobalUnlock(memory);
                if EmptyClipboard() == 0 || SetClipboardData(13, memory).is_null() {
                    GlobalFree(memory);
                    return Err("无法更新剪贴板，请重试。".into());
                }
                Ok(())
            })();
            CloseClipboard();
            DestroyWindow(owner);
            result
        }
    }
    #[cfg(not(windows))]
    {
        let _ = value;
        Err("此桌面版本只支持 Windows 剪贴板".into())
    }
}

pub fn message(text: &str, question: bool) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO,
        };
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide(text).as_ptr(),
                wide("物理世界的工具箱").as_ptr(),
                if question {
                    MB_YESNO | MB_ICONQUESTION
                } else {
                    MB_OK | MB_ICONINFORMATION
                },
            ) == IDYES
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("{text}");
        false
    }
}

pub fn replace_file(candidate: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
        let result = unsafe {
            ReplaceFileW(
                wide(target).as_ptr(),
                wide(candidate).as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(candidate, target).map_err(|e| e.to_string())
    }
}

pub fn rename_new(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
        let result = unsafe { MoveFileExW(wide(source).as_ptr(), wide(destination).as_ptr(), 0) };
        if result == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::hard_link(source, destination).map_err(|e| e.to_string())?;
        std::fs::remove_file(source).map_err(|e| e.to_string())
    }
}

/// A process handle opened before the parent can exit avoids PID reuse during handoff.
#[cfg(windows)]
pub struct ParentProcess(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ParentProcess {
    pub fn open(pid: u32) -> Result<Self, String> {
        let handle =
            unsafe { windows_sys::Win32::System::Threading::OpenProcess(0x0010_0000, 0, pid) };
        if handle.is_null() {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(Self(handle))
        }
    }
    pub fn wait(&self, timeout_ms: u32) -> Result<(), String> {
        let result = unsafe {
            windows_sys::Win32::System::Threading::WaitForSingleObject(self.0, timeout_ms)
        };
        if result == 0 {
            Ok(())
        } else {
            Err("旧工具箱仍在运行，未替换主程序。".into())
        }
    }
}
#[cfg(windows)]
impl Drop for ParentProcess {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
