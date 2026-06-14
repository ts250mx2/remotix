//! Tipos de los mensajes de señalización (deben coincidir con server/src/ws/signaling.ts
//! y web/src/helpdesk/connection.ts).

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(tag = "t")]
pub enum Outgoing {
    #[serde(rename = "host")]
    Host {
        name: Option<String>,
        mode: &'static str,
        caps: Vec<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    #[serde(rename = "signal")]
    Signal { payload: SignalPayload },
    #[serde(rename = "chat")]
    Chat { text: String },
    // Rol operador (visor nativo): unirse a una sala por código.
    #[serde(rename = "join")]
    Join { code: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "t")]
pub enum Incoming {
    #[serde(rename = "hosted")]
    Hosted {
        code: String,
        #[serde(default, rename = "iceServers")]
        ice_servers: Vec<IceServer>,
    },
    #[serde(rename = "peer-joined")]
    PeerJoined,
    #[serde(rename = "peer-left")]
    PeerLeft,
    // Respuesta a `join` (rol operador): el host está presente / aún no.
    #[serde(rename = "joined")]
    Joined {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        caps: Vec<String>,
    },
    #[serde(rename = "waiting")]
    Waiting,
    #[serde(rename = "signal")]
    Signal { payload: SignalPayload },
    #[serde(rename = "chat")]
    Chat {
        text: String,
        #[serde(default)]
        from: Option<String>,
    },
    #[serde(rename = "error")]
    Error { code: String },
    // Otros tipos que no usamos (hello, joined, etc.) caen aquí.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SignalPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp: Option<Sdp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<IceCandidate>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Sdp {
    #[serde(rename = "type")]
    pub kind: String,
    pub sdp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IceCandidate {
    pub candidate: String,
    #[serde(rename = "sdpMid", default, skip_serializing_if = "Option::is_none")]
    pub sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex", default, skip_serializing_if = "Option::is_none")]
    pub sdp_mline_index: Option<u16>,
    #[serde(rename = "usernameFragment", default, skip_serializing_if = "Option::is_none")]
    pub username_fragment: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct IceServer {
    #[serde(default)]
    pub urls: UrlsField,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

/// `urls` puede ser un string o un array de strings.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(untagged)]
pub enum UrlsField {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl UrlsField {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            UrlsField::None => Vec::new(),
            UrlsField::One(s) => vec![s],
            UrlsField::Many(v) => v,
        }
    }
}
