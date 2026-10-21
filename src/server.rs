//! Talking to the family's place server (photo-stamp-server, `/v1`). Every request carries
//! `Authorization: Bearer <device key>`.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::places::{GpsPoint, PlaceReply, PlaceRequest};

pub const VERSION: &str = env!("PSS_VERSION");

/// What `GET /v1/me` says about this device (the fields the settings dialog shows).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub user: String,
    pub device: String,
    pub server_version: String,
    /// When the server's place database was built (YYYY-MM-DD).
    #[serde(default)]
    pub places_built: Option<String>,
    /// The server's data release and this person's places revision: cached answers from other ones
    /// are asked again.
    #[serde(default)]
    pub data_version: String,
    #[serde(default)]
    pub rev: u64,
}

impl Me {
    pub fn summary(&self) -> String {
        let mut s = format!("Connected as {} ({}). Server {}", self.user, self.device, self.server_version);
        if let Some(built) = &self.places_built {
            s.push_str(&format!(", places from {built}"));
        }
        s.push('.');
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// 401/403: the server doesn't know this key.
    Refused,
    /// Answered, but not like a place server.
    Unexpected(String),
    Unreachable(String),
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckError::Refused => write!(f, "The server didn't accept this device key."),
            CheckError::Unexpected(what) => write!(f, "That doesn't look like a place server: {what}."),
            CheckError::Unreachable(why) => write!(f, "Couldn't reach the server: {why}."),
        }
    }
}

/// Asks the server who this key belongs to: proves the address and key work.
pub fn check(url: &str, key: &str, timeout: Duration) -> Result<Me, CheckError> {
    Client::new(url, key, timeout).me()
}

/// One place server and this PC's key for it.
pub struct Client {
    url: String,
    auth: String,
    agent: ureq::Agent,
}

impl Client {
    /// `timeout`: for each whole request.
    pub fn new(url: &str, key: &str, timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .user_agent(format!("photo-stamp-screensaver/{VERSION}"))
            .build()
            .into();
        Self { url: url.to_owned(), auth: format!("Bearer {key}"), agent }
    }

    pub fn me(&self) -> Result<Me, CheckError> {
        let resp = self.agent.get(format!("{}/v1/me", self.url)).header("Authorization", &self.auth).call();
        parse(resp, "/v1/me")
    }

    pub fn place(&self, at: GpsPoint) -> Result<PlaceReply, CheckError> {
        let body = serde_json::to_vec(&PlaceRequest { lat: at.lat, lon: at.lon }).unwrap_or_default();
        let resp = self
            .agent
            .post(format!("{}/v1/place", self.url))
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .send(&body[..]);
        parse(resp, "/v1/place")
    }
}

fn parse<T: serde::de::DeserializeOwned>(
    resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    what: &str,
) -> Result<T, CheckError> {
    let mut resp = resp.map_err(|e| CheckError::Unreachable(e.to_string()))?;
    let status = resp.status().as_u16();
    match status {
        200 => {}
        401 | 403 => return Err(CheckError::Refused),
        _ => return Err(CheckError::Unexpected(format!("HTTP {status}"))),
    }
    let body = resp.body_mut().read_to_string().map_err(|e| CheckError::Unreachable(e.to_string()))?;
    serde_json::from_str(&body).map_err(|_| CheckError::Unexpected(format!("its answer to {what}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// A one-request server that answers like photo-stamp-server would for `key`.
    fn fake_server(key: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut authorized = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                authorized |= line.eq_ignore_ascii_case(&format!("authorization: Bearer {key}\r\n"));
            }
            let (status, body) = if authorized {
                (
                    "200 OK",
                    r#"{"user":"dave","device":"den","history":true,"max_wait_hours":72,"rev":1,"data_version":"x","server_version":"1.2.3","places_built":"2026-10-11"}"#,
                )
            } else {
                ("401 Unauthorized", r#"{"error":"missing or unknown device key"}"#)
            };
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        url
    }

    #[test]
    fn right_key() {
        let me = check(&fake_server("psk_ok"), "psk_ok", Duration::from_secs(5)).unwrap();
        assert_eq!(me.summary(), "Connected as dave (den). Server 1.2.3, places from 2026-10-11.");
    }

    #[test]
    fn wrong_key() {
        assert_eq!(
            check(&fake_server("psk_ok"), "psk_nope", Duration::from_secs(5)),
            Err(CheckError::Refused)
        );
    }

    #[test]
    fn nobody_there() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = check(&format!("http://127.0.0.1:{port}"), "k", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CheckError::Unreachable(_)), "{err:?}");
    }
}
