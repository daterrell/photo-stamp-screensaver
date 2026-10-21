//! Talking to the family's place server (photo-stamp-server, `/v1`). Every request carries
//! `Authorization: Bearer <device key>`. The types are photo-stamp's (`proto`).

use std::fmt;
use std::time::Duration;

use crate::places::{ClientUpdate, GpsPoint, Me, PlaceReply, PlaceRequest};

pub const VERSION: &str = env!("PSS_VERSION");

/// What the settings dialog says after a connection test.
pub fn summary(me: &Me) -> String {
    let mut s = format!("Connected as {} ({}). Server {}", me.user, me.device, me.server_version);
    if let Some(built) = &me.places_built {
        s.push_str(&format!(", places from {built}"));
    }
    s.push('.');
    s
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

impl std::error::Error for CheckError {}

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
        json(&self.get("/v1/me")?, "/v1/me")
    }

    pub fn place(&self, at: GpsPoint) -> Result<PlaceReply, CheckError> {
        let body = serde_json::to_vec(&PlaceRequest { lat: at.lat, lon: at.lon, accuracy_m: None })
            .unwrap_or_default();
        let resp = self
            .agent
            .post(format!("{}/v1/place", self.url))
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .send(&body[..]);
        json(&body_of(resp)?, "/v1/place")
    }

    /// The screensaver's newest build on `channel` ("stable" or "beta"); `None` when none is
    /// published.
    pub fn latest(&self, channel: &str) -> Result<Option<ClientUpdate>, CheckError> {
        let path = format!("/v1/client/latest?app=screensaver&channel={channel}");
        let resp = self.agent.get(format!("{}{path}", self.url)).header("Authorization", &self.auth).call();
        let mut resp = resp.map_err(|e| CheckError::Unreachable(e.to_string()))?;
        if resp.status().as_u16() == 204 {
            return Ok(None);
        }
        let bytes = read(&mut resp)?;
        json(&bytes, "/v1/client/latest").map(Some)
    }

    /// Anything the server serves by path: a build to download, the towns database.
    pub fn get(&self, path: &str) -> Result<Vec<u8>, CheckError> {
        body_of(self.agent.get(format!("{}{path}", self.url)).header("Authorization", &self.auth).call())
    }
}

/// The body of a successful answer.
fn body_of(resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Vec<u8>, CheckError> {
    let mut resp = resp.map_err(|e| CheckError::Unreachable(e.to_string()))?;
    read(&mut resp)
}

fn read(resp: &mut ureq::http::Response<ureq::Body>) -> Result<Vec<u8>, CheckError> {
    match resp.status().as_u16() {
        200 => {}
        401 | 403 => return Err(CheckError::Refused),
        status => return Err(CheckError::Unexpected(format!("HTTP {status}"))),
    }
    resp.body_mut()
        .with_config()
        // Builds are ~20 MB, the towns database ~8.
        .limit(64 << 20)
        .read_to_vec()
        .map_err(|e| CheckError::Unreachable(e.to_string()))
}

fn json<T: serde::de::DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T, CheckError> {
    serde_json::from_slice(bytes).map_err(|_| CheckError::Unexpected(format!("its answer to {what}")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// What a fake server answers: status, then body.
    pub type Answer = (u16, Vec<u8>);

    /// A stand-in place server: `handle(method and path, authorized, body)` for each request, with
    /// `key` the only key it knows.
    pub fn fake_server(
        key: &'static str,
        handle: impl Fn(&str, bool, &[u8]) -> Answer + Send + 'static,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let (mut len, mut authorized) = (0, false);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    authorized |= line.eq_ignore_ascii_case(&format!("authorization: Bearer {key}\r\n"));
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; len];
                let _ = reader.read_exact(&mut body);
                let target = request.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
                let (status, reply) = handle(&target, authorized, &body);
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.len()
                );
                let _ = stream.write_all(&reply);
            }
        });
        url
    }

    pub const ME: &str = r#"{"user":"dave","device":"den","history":true,"max_wait_hours":72,"rev":1,"data_version":"d1","server_version":"1.2.3","places_built":"2026-10-11"}"#;

    fn me_server() -> String {
        fake_server("psk_ok", |_, authorized, _| {
            if authorized {
                (200, ME.into())
            } else {
                (401, br#"{"error":"missing or unknown device key"}"#.to_vec())
            }
        })
    }

    #[test]
    fn right_key() {
        let me = check(&me_server(), "psk_ok", Duration::from_secs(5)).unwrap();
        assert_eq!(summary(&me), "Connected as dave (den). Server 1.2.3, places from 2026-10-11.");
    }

    #[test]
    fn wrong_key() {
        assert_eq!(check(&me_server(), "psk_nope", Duration::from_secs(5)), Err(CheckError::Refused));
    }

    #[test]
    fn nobody_there() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = check(&format!("http://127.0.0.1:{port}"), "k", Duration::from_secs(5)).unwrap_err();
        assert!(matches!(err, CheckError::Unreachable(_)), "{err:?}");
    }

    #[test]
    fn latest_builds() {
        let url = fake_server("k", |target, _, _| match target {
            "GET /v1/client/latest?app=screensaver&channel=stable" => (
                200,
                br#"{"version":"0.2.0","sha256":"ab","path":"/v1/client/download/0.2.0?app=screensaver"}"#
                    .to_vec(),
            ),
            _ => (204, Vec::new()),
        });
        let client = Client::new(&url, "k", Duration::from_secs(5));
        assert_eq!(client.latest("stable").unwrap().unwrap().version, "0.2.0");
        assert_eq!(client.latest("beta").unwrap(), None);
    }
}
