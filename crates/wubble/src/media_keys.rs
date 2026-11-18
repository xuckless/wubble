//! Media keys and the system's media controls (MPRIS on Linux, Now Playing on macOS, SMTC on
//! Windows) through the `souvlaki` crate (MIT). wubble is the only owner: CEF's Alloy runtime has
//! no media-key handling of its own (Chromium's lives in `//chrome`). The controls are created on
//! the first tab that plays sound, not at start, so an idle wubble never takes the keys from
//! another player. Their events arrive on souvlaki's thread and become a [`MediaKey`] on the
//! Slint thread; the host sends it to the tab that plays (`host/videos.rs`).

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

/// A media key, as the host acts on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKey {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
}

impl MediaKey {
    /// The key a system event stands for; the rest (seek, volume, raise) are not handled.
    pub fn of(event: &MediaControlEvent) -> Option<MediaKey> {
        Some(match event {
            MediaControlEvent::Play => MediaKey::Play,
            MediaControlEvent::Pause => MediaKey::Pause,
            MediaControlEvent::Toggle => MediaKey::Toggle,
            MediaControlEvent::Next => MediaKey::Next,
            MediaControlEvent::Previous => MediaKey::Previous,
            MediaControlEvent::Stop => MediaKey::Stop,
            _ => return None,
        })
    }

    /// The page kit's action for the key (`kit.js` `act`): the page's own media-session
    /// handler if it set one, else the video itself.
    pub fn action(self) -> &'static str {
        match self {
            MediaKey::Play => "play",
            MediaKey::Pause => "pause",
            MediaKey::Toggle => "playpause",
            MediaKey::Next => "nexttrack",
            MediaKey::Previous => "previoustrack",
            MediaKey::Stop => "stop",
        }
    }
}

/// What the system's controls show: the playing page's title, artist, album and artwork.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The page's own `https` artwork URL; the desktop fetches it, wubble never does.
    pub art: String,
    pub playing: bool,
}

impl NowPlaying {
    /// The kit's `meta` object, its strings capped.
    /// The kit's `meta` (`kit.js` `snapshot()`): playing only when it says `paused: false`.
    pub fn from_json(v: &serde_json::Value) -> NowPlaying {
        let text = |k: &str| -> String {
            v.get(k)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(256)
                .collect()
        };
        let art = text("art");
        NowPlaying {
            title: text("title"),
            artist: text("artist"),
            album: text("album"),
            art: if art.starts_with("https://") {
                art
            } else {
                String::new()
            },
            playing: v.get("paused").and_then(serde_json::Value::as_bool) == Some(false),
        }
    }
}

/// The system's controls, once created.
pub struct Keys {
    controls: MediaControls,
    shown: Option<NowPlaying>,
}

impl Keys {
    /// Registers wubble with the system (`org.mpris.MediaPlayer2.wubble` on Linux). Each key
    /// goes to `pressed` on souvlaki's thread.
    pub fn start(pressed: impl Fn(MediaKey) + Send + 'static) -> Result<Keys, String> {
        let config = PlatformConfig {
            display_name: "wubble",
            dbus_name: "wubble",
            // Windows needs the window's HWND; macOS and Linux none. Windows is CI-only and
            // gets none yet: souvlaki refuses there, and the keys stay off.
            hwnd: None,
        };
        let mut controls = MediaControls::new(config).map_err(|e| format!("{e:?}"))?;
        controls
            .attach(move |event| {
                if let Some(key) = MediaKey::of(&event) {
                    pressed(key);
                }
            })
            .map_err(|e| format!("{e:?}"))?;
        Ok(Keys {
            controls,
            shown: None,
        })
    }

    /// Nothing plays: the system's controls show it stopped (once).
    pub fn stop(&mut self) {
        if self.shown.take().is_some()
            && let Err(e) = self.controls.set_playback(MediaPlayback::Stopped)
        {
            eprintln!("wubble: media controls: {e:?}");
        }
    }

    /// Shows `now` (only if it changed).
    pub fn show(&mut self, now: &NowPlaying) {
        if self.shown.as_ref() == Some(now) {
            return;
        }
        fn some(s: &str) -> Option<&str> {
            (!s.is_empty()).then_some(s)
        }
        let metadata = MediaMetadata {
            title: some(&now.title),
            artist: some(&now.artist),
            album: some(&now.album),
            cover_url: some(&now.art),
            duration: None,
        };
        let playback = if now.playing {
            MediaPlayback::Playing { progress: None }
        } else {
            MediaPlayback::Paused { progress: None }
        };
        if let Err(e) = self
            .controls
            .set_metadata(metadata)
            .and_then(|()| self.controls.set_playback(playback))
        {
            eprintln!("wubble: media controls: {e:?}");
        }
        self.shown = Some(now.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_events_become_keys_and_kit_actions() {
        let cases = [
            (MediaControlEvent::Play, Some("play")),
            (MediaControlEvent::Pause, Some("pause")),
            (MediaControlEvent::Toggle, Some("playpause")),
            (MediaControlEvent::Next, Some("nexttrack")),
            (MediaControlEvent::Previous, Some("previoustrack")),
            (MediaControlEvent::Stop, Some("stop")),
            (MediaControlEvent::Raise, None),
            (MediaControlEvent::SetVolume(0.5), None),
        ];
        for (event, want) in cases {
            assert_eq!(
                MediaKey::of(&event).map(MediaKey::action),
                want,
                "{event:?}"
            );
        }
    }

    #[test]
    fn page_metadata_is_capped_and_only_https_artwork_passes() {
        let v = serde_json::json!({
            "title": "t".repeat(300), "artist": "A", "album": 5,
            "art": "http://plain.test/a.png"
        });
        let now = NowPlaying::from_json(&v);
        assert_eq!(now.title.len(), 256);
        assert_eq!((now.artist.as_str(), now.album.as_str()), ("A", ""));
        assert_eq!(now.art, "");
        let v = serde_json::json!({"art": "https://img.test/a.png"});
        assert_eq!(NowPlaying::from_json(&v).art, "https://img.test/a.png");
        // A paused video shows paused, and so does a meta that does not say.
        let playing = |v: serde_json::Value| NowPlaying::from_json(&v).playing;
        assert!(playing(serde_json::json!({"paused": false})));
        assert!(!playing(serde_json::json!({"paused": true})));
        assert!(!playing(serde_json::json!({})));
    }
}
