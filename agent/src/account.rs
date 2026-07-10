//! Cliente REST autenticado del exe (rol operador): login del usuario, libreta de
//! PCs accesibles y reserva de sesión para conectarse. Mantiene el token de
//! sesión (cookie `remotix_session`) y lo envía manualmente en cada llamada.

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::config::to_http;

#[derive(Deserialize, Clone, Debug)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub access_key: String,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub role: String,
}

#[derive(Clone)]
pub struct Account {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl Account {
    pub fn new(server: &str, token: Option<String>) -> Self {
        Self { http: reqwest::Client::new(), base: to_http(server), token }
    }

    pub fn token(&self) -> Option<String> {
        self.token.clone()
    }
    pub fn is_logged_in(&self) -> bool {
        self.token.is_some()
    }
    fn cookie(&self) -> Option<String> {
        self.token.as_ref().map(|t| format!("remotix_session={t}"))
    }

    pub async fn login(&mut self, email: &str, password: &str) -> Result<UserInfo> {
        let url = format!("{}/api/auth/login", self.base);
        let resp = self.http.post(&url)
            .json(&serde_json::json!({ "email": email, "password": password }))
            .send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("credenciales inválidas ({})", resp.status().as_u16()));
        }
        let token = resp.headers().get_all(reqwest::header::SET_COOKIE).iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(|s| s.split(';').next().and_then(|kv| kv.trim().strip_prefix("remotix_session=")))
            .map(|t| t.to_string())
            .ok_or_else(|| anyhow!("el servidor no devolvió la cookie de sesión"))?;
        #[derive(Deserialize)]
        struct R { user: UserInfo }
        let r: R = resp.json().await?;
        self.token = Some(token);
        Ok(r.user)
    }

    /// Valida un token persistido. `Ok(Some)` = sesión viva; `Ok(None)` = el
    /// servidor la RECHAZÓ (401/403: borrar el token); `Err` = fallo de red u
    /// otro error transitorio (NO borrar el token: arrancar sin internet no
    /// debe desloguear al usuario).
    pub async fn me(&self) -> Result<Option<UserInfo>> {
        let url = format!("{}/api/auth/me", self.base);
        let mut req = self.http.get(&url);
        if let Some(c) = self.cookie() { req = req.header(reqwest::header::COOKIE, c); }
        let resp = req.send().await?;
        match resp.status().as_u16() {
            401 | 403 => Ok(None),
            s if !(200..300).contains(&s) => Err(anyhow!("respuesta inesperada ({s})")),
            _ => {
                #[derive(Deserialize)]
                struct R { user: UserInfo }
                Ok(Some(resp.json::<R>().await?.user))
            }
        }
    }

    /// Reclama este equipo (por su clave fija) para el usuario logueado.
    pub async fn claim(&self, access_key: &str) -> Result<()> {
        let url = format!("{}/api/devices/claim", self.base);
        let mut req = self.http.post(&url).json(&serde_json::json!({ "accessKey": access_key }));
        if let Some(c) = self.cookie() { req = req.header(reqwest::header::COOKIE, c); }
        let _ = req.send().await?;
        Ok(())
    }

    /// Libreta: PCs a los que el usuario tiene acceso.
    pub async fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let url = format!("{}/api/devices", self.base);
        let mut req = self.http.get(&url);
        if let Some(c) = self.cookie() { req = req.header(reqwest::header::COOKIE, c); }
        let resp = req.send().await?;
        if !resp.status().is_success() { return Err(anyhow!("no autorizado ({})", resp.status().as_u16())); }
        #[derive(Deserialize)]
        struct R { devices: Vec<DeviceInfo> }
        Ok(resp.json::<R>().await?.devices)
    }

    /// Reserva una sesión con el equipo (le ordena compartir) y devuelve el código
    /// de señalización con el que el visor se une.
    pub async fn connect(&self, device_id: &str) -> Result<String> {
        let url = format!("{}/api/devices/{}/connect", self.base, device_id);
        let mut req = self.http.post(&url);
        if let Some(c) = self.cookie() { req = req.header(reqwest::header::COOKIE, c); }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                403 => anyhow!("no tienes acceso a este equipo"),
                409 => anyhow!("el equipo no está en línea"),
                _ => anyhow!("no se pudo conectar ({})", status.as_u16()),
            });
        }
        #[derive(Deserialize)]
        struct R { code: String }
        Ok(resp.json::<R>().await?.code)
    }

    /// Conecta por la clave fija de un equipo (modo ad-hoc). Equipos sin dueño se
    /// aceptan solo con la clave; los que tienen dueño exigen acceso. Devuelve
    /// (código de señalización, nombre del equipo).
    pub async fn connect_by_key(&self, access_key: &str) -> Result<(String, String)> {
        let url = format!("{}/api/device/connect", self.base);
        let mut req = self.http.post(&url).json(&serde_json::json!({ "accessKey": access_key }));
        if let Some(c) = self.cookie() { req = req.header(reqwest::header::COOKIE, c); }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 => anyhow!("inicia sesión para conectarte"),
                403 => anyhow!("no tienes acceso a ese equipo"),
                404 => anyhow!("clave no encontrada"),
                409 => anyhow!("el equipo no está en línea"),
                _ => anyhow!("no se pudo conectar ({})", status.as_u16()),
            });
        }
        #[derive(Deserialize)]
        struct R { code: String, #[serde(default)] name: String }
        let r: R = resp.json().await?;
        Ok((r.code, r.name))
    }

    pub async fn logout(&mut self) -> Result<()> {
        let url = format!("{}/api/auth/logout", self.base);
        let mut req = self.http.post(&url);
        if let Some(c) = self.cookie() { req = req.header(reqwest::header::COOKIE, c); }
        let _ = req.send().await;
        self.token = None;
        Ok(())
    }
}
