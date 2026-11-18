//! The Shutdown Ritual's kiosk on Windows. The ritual window is topmost and full screen, and a
//! low-level keyboard hook swallows the keys that would switch away from it: the Windows keys,
//! Alt-Tab, Alt-Esc, Ctrl-Esc, Alt-F4 and Ctrl-Shift-Esc (Task Manager). Ctrl-Alt-Del can't be
//! hooked (it is the secure attention sequence); if the window is closed from there, the
//! service starts it again.
//!
//! [`swallow`] decides, purely; [`windows`] only installs the hook and raises the windows.

/// Virtual-key codes the decision looks at.
pub mod vk {
    pub const TAB: u32 = 0x09;
    pub const ESCAPE: u32 = 0x1B;
    pub const LWIN: u32 = 0x5B;
    pub const RWIN: u32 = 0x5C;
    pub const F4: u32 = 0x73;
}

/// Modifier keys held while a key is pressed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

/// Whether the hook swallows a key press of `key` with `modifiers` held.
pub fn swallow(key: u32, modifiers: Modifiers) -> bool {
    match key {
        vk::LWIN | vk::RWIN => true,
        vk::TAB => modifiers.alt,
        vk::ESCAPE => modifiers.alt || modifiers.ctrl,
        vk::F4 => modifiers.alt,
        _ => false,
    }
}

#[cfg(windows)]
pub mod windows {
    //! The hook and the topmost windows. Thin: every decision is [`super::swallow`].

    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_SHIFT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, EnumWindows, GetMessageW, GetWindowThreadProcessId, IsWindowVisible,
        PostThreadMessageW, SetForegroundWindow, SetWindowPos, SetWindowsHookExW,
        UnhookWindowsHookEx, HWND_TOPMOST, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, MSG, SWP_NOMOVE,
        SWP_NOSIZE, SWP_SHOWWINDOW, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    use super::{swallow, Modifiers};

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && (wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN) {
            // SAFETY: for WH_KEYBOARD_LL, lparam points to a KBDLLHOOKSTRUCT.
            let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } < 0;
            let modifiers = Modifiers {
                alt: info.flags & LLKHF_ALTDOWN != 0,
                ctrl: down(VK_CONTROL),
                shift: down(VK_SHIFT),
            };
            if swallow(info.vkCode, modifiers) {
                return 1;
            }
        }
        // SAFETY: passing the event on unchanged.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// Install the keyboard hook (once) on its own thread with a message loop, as low-level
    /// hooks require.
    pub fn install_keyboard_hook() {
        if INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| unsafe {
            // SAFETY: plain Win32 calls; the hook lives until this thread's loop ends.
            let module = GetModuleHandleW(std::ptr::null());
            let handle = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), module, 0);
            if handle.is_null() {
                INSTALLED.store(false, Ordering::SeqCst);
                return;
            }
            HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            UnhookWindowsHookEx(handle);
            INSTALLED.store(false, Ordering::SeqCst);
        });
    }

    /// Remove the hook (the process exiting does this too).
    pub fn remove_keyboard_hook() {
        let thread = HOOK_THREAD.swap(0, Ordering::SeqCst);
        if thread != 0 {
            // SAFETY: asking our own hook thread to leave its message loop.
            unsafe { PostThreadMessageW(thread, WM_QUIT, 0, 0) };
        }
    }

    unsafe extern "system" fn raise(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut pid = 0;
        // SAFETY: hwnd comes from EnumWindows.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                SetForegroundWindow(hwnd);
            }
        }
        1
    }

    /// Make every visible window of this process topmost and bring it to the front.
    pub fn raise_windows() {
        // SAFETY: the callback only touches windows EnumWindows hands it.
        unsafe { EnumWindows(Some(raise), 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        alt: false,
        ctrl: false,
        shift: false,
    };
    const ALT: Modifiers = Modifiers {
        alt: true,
        ctrl: false,
        shift: false,
    };
    const CTRL: Modifiers = Modifiers {
        alt: false,
        ctrl: true,
        shift: false,
    };
    const CTRL_SHIFT: Modifiers = Modifiers {
        alt: false,
        ctrl: true,
        shift: true,
    };

    #[test]
    fn the_windows_keys_never_reach_the_shell() {
        assert!(swallow(vk::LWIN, NONE));
        assert!(swallow(vk::RWIN, CTRL));
    }

    #[test]
    fn switching_and_closing_chords_are_swallowed() {
        assert!(swallow(vk::TAB, ALT), "Alt-Tab");
        assert!(swallow(vk::ESCAPE, ALT), "Alt-Esc");
        assert!(swallow(vk::ESCAPE, CTRL), "Ctrl-Esc (Start)");
        assert!(
            swallow(vk::ESCAPE, CTRL_SHIFT),
            "Ctrl-Shift-Esc (Task Manager)"
        );
        assert!(swallow(vk::F4, ALT), "Alt-F4");
    }

    #[test]
    fn typing_in_the_ritual_still_works() {
        assert!(!swallow(vk::TAB, NONE), "Tab moves between fields");
        assert!(!swallow(vk::ESCAPE, NONE));
        assert!(!swallow(0x41, NONE), "A");
        assert!(
            !swallow(
                0x41,
                Modifiers {
                    shift: true,
                    ..NONE
                }
            ),
            "Shift-A"
        );
        assert!(!swallow(0x56, CTRL), "Ctrl-V pastes");
        assert!(!swallow(vk::F4, NONE));
    }
}
