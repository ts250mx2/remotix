//! Ayudante en modo bandeja (sin ventana). Lo lanza el servicio en la sesión
//! interactiva: muestra un icono en la bandeja con la clave y el estado, y
//! hospeda las sesiones remotas. No abre ninguna ventana visible ni es cerrable
//! por el usuario (es un host desatendido: solo lo detiene el servicio).
//!
//! En la pantalla de login corre como SYSTEM en el escritorio seguro, donde no
//! hay shell: si la bandeja no puede crearse, el proceso sigue vivo sin icono
//! para conservar la presencia y poder hospedar.
#![cfg(windows)]

use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, SetTimer, TranslateMessage, MSG, WM_TIMER,
};

use crate::session::LiteEvent;

/// Formatea la clave en grupos de 3 (123-456-789) para leerla mejor.
fn format_key(k: &str) -> String {
    k.chars()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Punto de entrada del ayudante: corre el dispositivo (presencia + hosting) en
/// un runtime tokio y la bandeja en el hilo principal.
pub fn run_helper(server: String, name: String) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let (tx, rx) = std::sync::mpsc::channel::<LiteEvent>();
    rt.spawn(crate::device::run_helper_device(server, name, tx));

    // La bandeja debe vivir en el hilo principal (bombea la cola de mensajes).
    // Si no puede crearse (escritorio seguro/login sin shell), el proceso sigue
    // vivo sin icono: el dispositivo corre en los hilos de tokio igualmente.
    if let Err(e) = run_tray(rx) {
        tracing::warn!("bandeja no disponible ({e}); ejecutando sin icono");
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
    Ok(())
}

fn run_tray(rx: Receiver<LiteEvent>) -> Result<()> {
    let menu = Menu::new();
    // Elementos informativos (deshabilitados): el host no es cerrable a mano.
    let status_item = MenuItem::new("Iniciando…", false, None);
    let key_item = MenuItem::new("Clave: —", false, None);
    let ver_item = MenuItem::new(format!("Versión {}", crate::update::CURRENT_VERSION), false, None);
    menu.append(&status_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&key_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&ver_item)?;

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("Remotix v{} — Acceso remoto", crate::update::CURRENT_VERSION))
        .with_icon(make_icon())
        .build()?;

    unsafe {
        // Timer de thread (hwnd nulo): despierta el bucle cada 250 ms para
        // refrescar la bandeja con el estado/clave del dispositivo.
        let timer = SetTimer(std::ptr::null_mut(), 1, 250, None);
        let mut msg: MSG = std::mem::zeroed();

        // Bucle de mensajes: vive hasta que el servicio termine este proceso.
        loop {
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r == 0 || r == -1 {
                break;
            }
            if msg.message == WM_TIMER {
                while let Ok(ev) = rx.try_recv() {
                    match ev {
                        LiteEvent::Code(c) => {
                            let _ = key_item.set_text(format!("Clave: {}", format_key(&c)));
                        }
                        LiteEvent::Status(s) => {
                            let _ = status_item.set_text(s);
                        }
                        // La actualización del host la aplica el SERVICIO (canal
                        // host); el ayudante no se auto-actualiza.
                        LiteEvent::UpdateAvailable => {}
                    }
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        KillTimer(std::ptr::null_mut(), timer);
    }

    Ok(())
}

/// Icono generado en código (sin assets externos): disco azul degradado con un
/// punto ámbar en el centro, en línea con la identidad de Remotix.
fn make_icon() -> Icon {
    const S: usize = 32;
    let mut rgba = vec![0u8; S * S * 4];
    let (cx, cy, r) = (15.5f32, 15.5f32, 15.5f32);
    for y in 0..S {
        for x in 0..S {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            let i = (y * S + x) * 4;
            if d <= r {
                let t = y as f32 / S as f32;
                rgba[i] = (40.0 + 20.0 * t) as u8;
                rgba[i + 1] = (120.0 + 40.0 * t) as u8;
                rgba[i + 2] = (230.0 - 20.0 * t) as u8;
                rgba[i + 3] = 255;
            }
        }
    }
    let (ox, oy, orr) = (16.0f32, 16.0f32, 5.0f32);
    for y in 0..S {
        for x in 0..S {
            let (dx, dy) = (x as f32 - ox, y as f32 - oy);
            if (dx * dx + dy * dy).sqrt() <= orr {
                let i = (y * S + x) * 4;
                rgba[i] = 255;
                rgba[i + 1] = 205;
                rgba[i + 2] = 100;
                rgba[i + 3] = 255;
            }
        }
    }
    Icon::from_rgba(rgba, S as u32, S as u32).expect("icono válido")
}
