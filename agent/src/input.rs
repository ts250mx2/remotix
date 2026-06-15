//! Recepción de eventos de control (mouse/teclado) e inyección vía enigo.
//! Los eventos llegan como JSON por el DataChannel 'control' y se procesan en un
//! hilo dedicado que posee el `Enigo` (evita problemas de Send/Sync).

use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use enigo::{
    Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{error, warn};

/// Rect del monitor activo: (left, top, width, height) en el escritorio virtual.
pub type MonitorRect = Arc<Mutex<(i32, i32, i32, i32)>>;

// `Serialize` además de `Deserialize` para que el rol operador (visor nativo)
// emita exactamente el mismo formato que consume el host.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "k")]
pub enum InputEvent {
    #[serde(rename = "move")]
    Move { x: f64, y: f64 },
    #[serde(rename = "down")]
    Down { x: f64, y: f64, button: i32 },
    #[serde(rename = "up")]
    Up { x: f64, y: f64, button: i32 },
    #[serde(rename = "wheel")]
    Wheel { x: f64, y: f64, dx: f64, dy: f64 },
    #[serde(rename = "key")]
    Key { down: bool, code: String, key: String },
}

/// Lanza el hilo de inyección y devuelve el canal por el que enviarle eventos.
/// `rect` es el rect (left, top, width, height) del monitor que se está viendo;
/// se actualiza al cambiar de monitor para mapear las coords 0..1 a su región.
pub fn spawn_injector(rect: MonitorRect) -> Sender<InputEvent> {
    let (tx, rx) = channel::<InputEvent>();
    std::thread::Builder::new()
        .name("input-injector".into())
        .spawn(move || {
            let mut enigo = match Enigo::new(&Settings::default()) {
                Ok(e) => e,
                Err(e) => {
                    error!("no se pudo inicializar Enigo: {e}");
                    return;
                }
            };
            while let Ok(evt) = rx.recv() {
                let (l, t, w, h) = *rect.lock();
                if let Err(e) = inject(&mut enigo, &evt, l, t, w, h) {
                    warn!("fallo inyectando {evt:?}: {e}");
                }
            }
        })
        .expect("spawn input thread");
    tx
}

fn to_px(v: f64, off: i32, span: i32) -> i32 {
    off + (v.clamp(0.0, 1.0) * span as f64).round() as i32
}

fn inject(enigo: &mut Enigo, evt: &InputEvent, l: i32, t: i32, w: i32, h: i32) -> enigo::InputResult<()> {
    match evt {
        InputEvent::Move { x, y } => {
            enigo.move_mouse(to_px(*x, l, w), to_px(*y, t, h), Coordinate::Abs)
        }
        InputEvent::Down { x, y, button } => {
            enigo.move_mouse(to_px(*x, l, w), to_px(*y, t, h), Coordinate::Abs)?;
            enigo.button(map_button(*button), Direction::Press)
        }
        InputEvent::Up { x, y, button } => {
            enigo.move_mouse(to_px(*x, l, w), to_px(*y, t, h), Coordinate::Abs)?;
            enigo.button(map_button(*button), Direction::Release)
        }
        InputEvent::Wheel { dy, dx, .. } => {
            let v = (*dy / 100.0).round() as i32;
            let hh = (*dx / 100.0).round() as i32;
            if v != 0 {
                enigo.scroll(v, Axis::Vertical)?;
            }
            if hh != 0 {
                enigo.scroll(hh, Axis::Horizontal)?;
            }
            Ok(())
        }
        InputEvent::Key { down, code, key } => {
            let dir = if *down { Direction::Press } else { Direction::Release };
            match map_key(code, key) {
                Some(k) => enigo.key(k, dir),
                None => Ok(()),
            }
        }
    }
}

fn map_button(button: i32) -> Button {
    match button {
        1 => Button::Middle,
        2 => Button::Right,
        _ => Button::Left,
    }
}

/// Mapea `KeyboardEvent.code`/`.key` del navegador a una tecla de enigo.
fn map_key(code: &str, key: &str) -> Option<Key> {
    // Teclas con nombre primero (por `code`, físico).
    let named = match code {
        "Enter" | "NumpadEnter" => Some(Key::Return),
        "Backspace" => Some(Key::Backspace),
        "Tab" => Some(Key::Tab),
        "Space" => Some(Key::Space),
        "Escape" => Some(Key::Escape),
        "Delete" => Some(Key::Delete),
        "Home" => Some(Key::Home),
        "End" => Some(Key::End),
        "PageUp" => Some(Key::PageUp),
        "PageDown" => Some(Key::PageDown),
        "ArrowUp" => Some(Key::UpArrow),
        "ArrowDown" => Some(Key::DownArrow),
        "ArrowLeft" => Some(Key::LeftArrow),
        "ArrowRight" => Some(Key::RightArrow),
        "ShiftLeft" | "ShiftRight" => Some(Key::Shift),
        "ControlLeft" | "ControlRight" => Some(Key::Control),
        "AltLeft" | "AltRight" => Some(Key::Alt),
        "MetaLeft" | "MetaRight" => Some(Key::Meta),
        "CapsLock" => Some(Key::CapsLock),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if let Some(f) = code.strip_prefix('F') {
        if let Ok(n) = f.parse::<u32>() {
            if (1..=12).contains(&n) {
                return Some(Key::F1.shifted_f(n));
            }
        }
    }
    // Tecla imprimible (un solo carácter en `key`).
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next()) {
        if !c.is_control() {
            return Some(Key::Unicode(c));
        }
    }
    None
}

// Pequeña ayuda: F1..F12 a partir de un offset. (enigo expone F1..F35 como variantes.)
trait FKey {
    fn shifted_f(self, n: u32) -> Key;
}
impl FKey for Key {
    fn shifted_f(self, n: u32) -> Key {
        match n {
            1 => Key::F1,
            2 => Key::F2,
            3 => Key::F3,
            4 => Key::F4,
            5 => Key::F5,
            6 => Key::F6,
            7 => Key::F7,
            8 => Key::F8,
            9 => Key::F9,
            10 => Key::F10,
            11 => Key::F11,
            _ => Key::F12,
        }
    }
}
