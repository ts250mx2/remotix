//! Rects de los monitores físicos en el escritorio virtual (Win32), para mapear
//! las coordenadas normalizadas del operador al monitor que se está viendo.

/// (left, top, width, height) de cada monitor, en píxeles del escritorio virtual.
#[cfg(windows)]
pub fn monitor_rects() -> Vec<(i32, i32, i32, i32)> {
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

    unsafe extern "system" fn cb(_h: HMONITOR, _hdc: HDC, rect: *mut RECT, data: LPARAM) -> i32 {
        let v = &mut *(data as *mut Vec<(i32, i32, i32, i32)>);
        let r = &*rect;
        v.push((r.left, r.top, r.right - r.left, r.bottom - r.top));
        1 // continuar
    }

    let mut v: Vec<(i32, i32, i32, i32)> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut::<c_void>() as HDC,
            std::ptr::null(),
            Some(cb),
            &mut v as *mut _ as LPARAM,
        );
    }
    v
}

#[cfg(not(windows))]
pub fn monitor_rects() -> Vec<(i32, i32, i32, i32)> {
    Vec::new()
}

/// Rect del monitor que corresponde a la captura `idx` (de `scrap`) con tamaño
/// `scrap_w x scrap_h`. Empareja por dimensiones (más robusto que por orden) y
/// cae a (0,0,scrap_w,scrap_h) si no hay datos.
pub fn rect_for(idx: usize, scrap_w: i32, scrap_h: i32) -> (i32, i32, i32, i32) {
    let rects = monitor_rects();
    if rects.is_empty() {
        return (0, 0, scrap_w, scrap_h);
    }
    // 1) mismo índice si las dimensiones coinciden
    if let Some(r) = rects.get(idx) {
        if r.2 == scrap_w && r.3 == scrap_h {
            return *r;
        }
    }
    // 2) primer monitor con esas dimensiones
    if let Some(r) = rects.iter().find(|r| r.2 == scrap_w && r.3 == scrap_h) {
        return *r;
    }
    // 3) por índice, o el primero
    *rects.get(idx).unwrap_or(&rects[0])
}
