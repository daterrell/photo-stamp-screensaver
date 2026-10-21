//! What the settings dialog sets. Stored per user in the registry (`win::store`), with the device
//! key encrypted for this user.

use serde::{Deserialize, Serialize};

pub const MIN_SECONDS: u32 = 3;
pub const MAX_SECONDS: u32 = 3600;
pub const DEFAULT_SECONDS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The top folder of photos; all its subfolders are included. Empty: not chosen yet.
    pub folder: String,
    /// `https://host`, without a trailing slash. Empty: no place server (photos show without places).
    pub server_url: String,
    /// This PC's device key (`photo-stamp-server device add …`).
    pub device_key: String,
    pub seconds_per_photo: u32,
    pub caption: Caption,
    pub corner: Corner,
    pub order: Order,
    /// Which builds automatic updates install.
    pub channel: Channel,
}

/// Which builds automatic updates install.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    /// Tagged releases only.
    #[default]
    Releases,
    /// Every build of main (beta).
    Every,
}

impl Channel {
    /// In the order of the dialog's list; the position is what's saved.
    pub const ALL: [(Channel, &str); 2] =
        [(Channel::Releases, "Releases only"), (Channel::Every, "Every build")];

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|(c, _)| *c == self).unwrap_or(0) as u32
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).map_or_else(Self::default, |(c, _)| *c)
    }

    /// What the place server calls it.
    pub fn server_name(self) -> &'static str {
        match self {
            Channel::Releases => "stable",
            Channel::Every => "beta",
        }
    }
}

/// What the updater (running as SYSTEM) uses: kept machine-wide, where only an administrator can
/// change it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Updates {
    pub server_url: String,
    pub device_key: String,
    pub channel: Channel,
}

/// Which photo comes next.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Order {
    /// Random, every photo once before any repeats.
    #[default]
    Shuffle,
    ByDate,
    /// Folder by folder, by name within each.
    ByFolder,
}

/// Whether where and when a photo was taken shows over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Caption {
    #[default]
    Always,
    /// Fades in a moment after the photo appears, then out again before the next.
    Fade,
    Hidden,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Corner {
    #[default]
    BottomLeft,
    BottomRight,
    TopLeft,
    TopRight,
}

impl Caption {
    /// In the order of the dialog's list; the position is what's saved.
    pub const ALL: [(Caption, &str); 3] =
        [(Caption::Always, "Always"), (Caption::Fade, "Fading in and out"), (Caption::Hidden, "Never")];

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|(c, _)| *c == self).unwrap_or(0) as u32
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).map_or_else(Self::default, |(c, _)| *c)
    }
}

impl Order {
    /// In the order of the dialog's list; the position is what's saved.
    pub const ALL: [(Order, &str); 3] =
        [(Order::Shuffle, "Shuffled"), (Order::ByDate, "By date taken"), (Order::ByFolder, "By folder")];

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|(c, _)| *c == self).unwrap_or(0) as u32
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).map_or_else(Self::default, |(c, _)| *c)
    }
}

impl Corner {
    /// In the order of the dialog's list; the position is what's saved.
    pub const ALL: [(Corner, &str); 4] = [
        (Corner::BottomLeft, "Bottom left"),
        (Corner::BottomRight, "Bottom right"),
        (Corner::TopLeft, "Top left"),
        (Corner::TopRight, "Top right"),
    ];

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|(c, _)| *c == self).unwrap_or(0) as u32
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).map_or_else(Self::default, |(c, _)| *c)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: String::new(),
            server_url: String::new(),
            device_key: String::new(),
            seconds_per_photo: DEFAULT_SECONDS,
            caption: Caption::default(),
            corner: Corner::default(),
            order: Order::default(),
            channel: Channel::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Folder,
    ServerUrl,
    DeviceKey,
    Seconds,
}

/// Something typed in the dialog that can't be saved, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub field: Field,
    pub message: String,
}

impl Settings {
    /// The update settings these imply; `None` without a place server.
    pub fn updates(&self) -> Option<Updates> {
        (!self.server_url.is_empty()).then(|| Updates {
            server_url: self.server_url.clone(),
            device_key: self.device_key.clone(),
            channel: self.channel,
        })
    }

    /// Tidied (trimmed, normalized address) and checked, ready to save.
    pub fn validated(self) -> Result<Settings, Invalid> {
        let invalid = |field, message: &str| Invalid { field, message: message.to_owned() };
        let folder = self.folder.trim().to_owned();
        let device_key = self.device_key.trim().to_owned();
        let server_url = match self.server_url.trim() {
            "" if device_key.is_empty() => String::new(),
            "" => {
                return Err(invalid(
                    Field::ServerUrl,
                    "Enter the place server's address, or clear the device key.",
                ));
            }
            url => normalize_url(url).map_err(|m| invalid(Field::ServerUrl, &m))?,
        };
        if !server_url.is_empty() && device_key.is_empty() {
            return Err(invalid(Field::DeviceKey, "Enter this PC's device key for the place server."));
        }
        if !(MIN_SECONDS..=MAX_SECONDS).contains(&self.seconds_per_photo) {
            return Err(Invalid {
                field: Field::Seconds,
                message: format!("Show each photo for {MIN_SECONDS} to {MAX_SECONDS} seconds."),
            });
        }
        Ok(Settings { folder, server_url, device_key, ..self })
    }
}

/// `https://host` or `http://host:port`, without a trailing slash (as photo-stamp has it).
pub fn normalize_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() < 10 {
        return Err(
            "The place server's address should start with https:// (or http:// on a private network).".into(),
        );
    }
    Ok(url.to_owned())
}

/// The place server photo-stamp uses on this PC, from its `client.json`
/// (`%LOCALAPPDATA%\photo-stamp\client.json`): a starting point the first time settings open.
pub fn photo_stamp_server(client_json: &[u8]) -> Option<(String, String)> {
    #[derive(Deserialize)]
    struct Client {
        server: Option<Server>,
    }
    #[derive(Deserialize)]
    struct Server {
        url: String,
        key: String,
    }
    let server = serde_json::from_slice::<Client>(client_json).ok()?.server?;
    Some((normalize_url(&server.url).ok()?, server.key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(url: &str, key: &str) -> Settings {
        Settings { server_url: url.into(), device_key: key.into(), ..Default::default() }
    }

    #[test]
    fn tidies() {
        let s = Settings {
            folder: "  D:\\Photos ".into(),
            server_url: " https://places.example.com/ ".into(),
            device_key: " psk_1 ".into(),
            seconds_per_photo: 8,
            caption: Caption::Fade,
            corner: Corner::TopRight,
            order: Order::ByDate,
            channel: Channel::Every,
        };
        assert_eq!(
            s.validated(),
            Ok(Settings {
                folder: "D:\\Photos".into(),
                server_url: "https://places.example.com".into(),
                device_key: "psk_1".into(),
                seconds_per_photo: 8,
                caption: Caption::Fade,
                corner: Corner::TopRight,
                order: Order::ByDate,
                channel: Channel::Every,
            })
        );
    }

    #[test]
    fn server_is_optional_but_complete() {
        assert!(settings("", "").validated().is_ok());
        assert_eq!(settings("", "psk_1").validated().unwrap_err().field, Field::ServerUrl);
        assert_eq!(
            settings("https://places.example.com", " ").validated().unwrap_err().field,
            Field::DeviceKey
        );
        assert_eq!(settings("places.example.com", "psk_1").validated().unwrap_err().field, Field::ServerUrl);
    }

    #[test]
    fn seconds_in_range() {
        for (secs, ok) in [(2, false), (3, true), (3600, true), (3601, false)] {
            let s = Settings { seconds_per_photo: secs, ..Default::default() };
            assert_eq!(s.validated().is_ok(), ok, "{secs}");
        }
    }

    #[test]
    fn choices_round_trip() {
        for (c, _) in Caption::ALL {
            assert_eq!(Caption::from_index(c.index()), c);
        }
        for (c, _) in Corner::ALL {
            assert_eq!(Corner::from_index(c.index()), c);
        }
        for (o, _) in Order::ALL {
            assert_eq!(Order::from_index(o.index()), o);
        }
        for (c, _) in Channel::ALL {
            assert_eq!(Channel::from_index(c.index()), c);
        }
        assert_eq!(Channel::default().server_name(), "stable");
        assert_eq!(Caption::from_index(99), Caption::Always);
        assert_eq!(Corner::from_index(99), Corner::BottomLeft);
    }

    #[test]
    fn reads_photo_stamps_server() {
        let json = br#"{"server":{"url":"https://places.example.com/","key":"psk_9"},"channel":"beta"}"#;
        assert_eq!(photo_stamp_server(json), Some(("https://places.example.com".into(), "psk_9".into())));
        assert_eq!(photo_stamp_server(br#"{"server":null}"#), None);
        assert_eq!(photo_stamp_server(b"not json"), None);
    }
}
