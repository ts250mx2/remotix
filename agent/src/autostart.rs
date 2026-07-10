//! Arranque con Windows (clave Run del registro de usuario actual).

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE: &str = "RemotixLite";

#[cfg(windows)]
pub fn set_autostart(enabled: bool) -> anyhow::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enabled {
        let exe = std::env::current_exe()?;
        // --tray: al iniciar sesión en Windows arranca oculto en la bandeja
        // (accesible por su clave), sin plantar la ventana en el escritorio.
        run.set_value(VALUE, &format!("\"{}\" --tray", exe.to_string_lossy()))?;
    } else {
        let _ = run.delete_value(VALUE);
    }
    Ok(())
}

#[cfg(windows)]
pub fn is_autostart() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>(VALUE).ok())
        .is_some()
}

#[cfg(not(windows))]
pub fn set_autostart(_enabled: bool) -> anyhow::Result<()> { Ok(()) }
#[cfg(not(windows))]
pub fn is_autostart() -> bool { false }
