//! Vincular el hilo actual al escritorio de ENTRADA activo (Windows).
//!
//! DXGI Desktop Duplication captura el escritorio del hilo que la crea, y la
//! inyección de input (SendInput) solo llega al escritorio de entrada. En la
//! pantalla de bloqueo/login o durante un aviso UAC, el escritorio de entrada es
//! `Winlogon` (o el escritorio seguro), distinto del `Default` del usuario. Un
//! ayudante SYSTEM que no se re-vincula captura NEGRO y no inyecta nada — ese es
//! el síntoma "pantalla negra en la pantalla de inicio de Windows".
//!
//! `DesktopBinder` recuerda a qué escritorio está vinculado el hilo y, cuando el
//! escritorio de entrada cambia (login↔escritorio, aparición de un UAC), re-vincula
//! el hilo. Los hilos de captura e inyección lo consultan para recrear entonces
//! sus recursos (el Capturer DXGI queda inválido al cambiar de escritorio).

#[cfg(windows)]
mod imp {
    use tracing::info;
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop, UOI_NAME,
    };

    // Todos los derechos específicos de escritorio (read/write/enumerate/switch…).
    const DESKTOP_ALL: u32 = 0x01FF;

    unsafe fn desktop_name(hdesk: *mut core::ffi::c_void) -> String {
        let mut buf = [0u16; 128];
        let mut needed = 0u32;
        let ok = GetUserObjectInformationW(
            hdesk,
            UOI_NAME as i32,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            (buf.len() * 2) as u32,
            &mut needed,
        );
        if ok == 0 {
            return String::new();
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    pub struct DesktopBinder {
        current: *mut core::ffi::c_void,
        name: String,
    }

    // Los HDESK son handles del proceso; el binder se mueve a un hilo dedicado.
    unsafe impl Send for DesktopBinder {}

    impl DesktopBinder {
        pub fn new() -> Self {
            Self { current: core::ptr::null_mut(), name: String::new() }
        }

        /// Re-vincula el hilo actual al escritorio de entrada si cambió desde la
        /// última vez. Devuelve `true` si cambió (el llamador debe recrear sus
        /// recursos ligados al escritorio). Best-effort: si no puede abrir/asignar
        /// el escritorio (p. ej. un usuario normal no puede abrir el seguro), deja
        /// el hilo donde está y devuelve `false`.
        pub fn rebind_if_changed(&mut self) -> bool {
            unsafe {
                let hdesk = OpenInputDesktop(0, 0, DESKTOP_ALL);
                if hdesk.is_null() {
                    return false;
                }
                let name = desktop_name(hdesk);
                if !self.current.is_null() && name == self.name {
                    CloseDesktop(hdesk); // ya estamos en él: descartamos el handle nuevo
                    return false;
                }
                if SetThreadDesktop(hdesk) == 0 {
                    CloseDesktop(hdesk);
                    return false;
                }
                if !self.current.is_null() {
                    CloseDesktop(self.current);
                }
                info!("hilo vinculado al escritorio de entrada '{name}'");
                self.current = hdesk;
                self.name = name;
                true
            }
        }

        pub fn name(&self) -> &str {
            &self.name
        }
    }

    impl Drop for DesktopBinder {
        fn drop(&mut self) {
            unsafe {
                if !self.current.is_null() {
                    CloseDesktop(self.current);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// Stub no-op fuera de Windows (el concepto de "escritorio de entrada" no aplica).
    pub struct DesktopBinder;
    impl DesktopBinder {
        pub fn new() -> Self {
            Self
        }
        pub fn rebind_if_changed(&mut self) -> bool {
            false
        }
        pub fn name(&self) -> &str {
            ""
        }
    }
}

pub use imp::DesktopBinder;
