//! The user's settings, kept as JSON in the user's config directory.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::send::{Command, Mode};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where the bar sits on Claude's window.
    pub position: Anchor,
    /// The buttons, in order.
    pub commands: Vec<Command>,
    /// The file on disk could not be read; saving would overwrite whatever the user wrote there.
    #[serde(skip)]
    unreadable: bool,
}

impl Default for Config {
    fn default() -> Config {
        let command = |text: &str| Command {
            text: text.into(),
            label: None,
            mode: Mode::Send,
        };
        Config {
            position: Anchor::default(),
            commands: vec![command("/compact"), command("/clear")],
            unreadable: false,
        }
    }
}

/// The bar's place relative to Claude's window: the bar's `corner` is `x`, `y`
/// away from the same corner of Claude's window, in 96-dpi pixels.
/// Pinning the nearest corner keeps the bar in place when Claude is resized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub corner: Corner,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub fn new(right: bool, bottom: bool) -> Corner {
        match (right, bottom) {
            (false, false) => Corner::TopLeft,
            (true, false) => Corner::TopRight,
            (false, true) => Corner::BottomLeft,
            (true, true) => Corner::BottomRight,
        }
    }

    pub fn right(self) -> bool {
        matches!(self, Corner::TopRight | Corner::BottomRight)
    }

    pub fn bottom(self) -> bool {
        matches!(self, Corner::BottomLeft | Corner::BottomRight)
    }
}

impl Default for Anchor {
    /// In Claude's title bar, left of the window buttons.
    fn default() -> Anchor {
        Anchor {
            corner: Corner::TopRight,
            x: -160,
            y: 4,
        }
    }
}

/// A window's bounds in screen pixels; `right` and `bottom` lie just outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Anchor {
    /// The anchor that keeps a bar at `bar` on `claude`, pinned to the nearest corner.
    /// Both are in screen pixels; `dpi` is Claude's.
    pub fn pin(bar: Rect, claude: Rect, dpi: u32) -> Anchor {
        let right = bar.left + bar.right > claude.left + claude.right;
        let bottom = bar.top + bar.bottom > claude.top + claude.bottom;
        let x = if right {
            bar.right - claude.right
        } else {
            bar.left - claude.left
        };
        let y = if bottom {
            bar.bottom - claude.bottom
        } else {
            bar.top - claude.top
        };
        Anchor {
            corner: Corner::new(right, bottom),
            x: logical(x, dpi),
            y: logical(y, dpi),
        }
    }

    /// The top-left corner `(x, y)` of a bar `width` by `height` anchored on `claude`;
    /// the inverse of [`Anchor::pin`].
    pub fn place(self, width: i32, height: i32, claude: Rect, dpi: u32) -> (i32, i32) {
        let (x, y) = (physical(self.x, dpi), physical(self.y, dpi));
        (
            if self.corner.right() {
                claude.right + x - width
            } else {
                claude.left + x
            },
            if self.corner.bottom() {
                claude.bottom + y - height
            } else {
                claude.top + y
            },
        )
    }
}

impl Config {
    /// The saved config; defaults when there is none or it cannot be read.
    pub fn load() -> Config {
        match path() {
            Some(path) => Config::load_from(&path),
            None => Config::default(),
        }
    }

    /// Writes the defaults out when there is no file, and adds the command list
    /// when the file has none, so the user has something to edit.
    fn load_from(path: &Path) -> Config {
        let json = match std::fs::read(path) {
            Ok(json) => json,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                let config = Config::default();
                // The bar works without the file; there is just nothing to edit yet.
                let _ = config.save_to(path);
                return config;
            }
            Err(_) => return Config::unreadable(),
        };
        // Serde would also take a bare array as a config; only an object is one.
        let Ok(value @ serde_json::Value::Object(_)) = serde_json::from_slice(&json) else {
            return Config::unreadable();
        };
        let has_commands = value.get("commands").is_some();
        let Ok(config) = Config::deserialize(value) else {
            return Config::unreadable();
        };
        if !has_commands {
            let _ = config.save_to(path);
        }
        config
    }

    fn unreadable() -> Config {
        Config {
            unreadable: true,
            ..Config::default()
        }
    }

    /// The file on disk was there but could not be read; these are the defaults.
    pub fn is_unreadable(&self) -> bool {
        self.unreadable
    }

    /// Fails without touching the file when it could not be read at startup.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&path().ok_or(ErrorKind::NotFound)?)
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if self.unreadable {
            return Err(ErrorKind::InvalidData.into());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        // Write aside and swap in, so a crash never leaves half a file.
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json)?;
        std::fs::rename(temp, path)
    }
}

/// `%APPDATA%\quickbar\config.json`.
fn path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("quickbar").join("config.json"))
}

/// 96-dpi pixels to pixels at `dpi`.
fn physical(px: i32, dpi: u32) -> i32 {
    (px as f64 * dpi as f64 / 96.0).round() as i32
}

/// Pixels at `dpi` to 96-dpi pixels.
fn logical(px: i32, dpi: u32) -> i32 {
    (px as f64 * 96.0 / dpi.max(1) as f64).round() as i32
}

#[cfg(test)]
mod format;

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory per test, removed when the test is done.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir = std::env::temp_dir().join(format!("quickbar-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            TempDir(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("config.json")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn pin_then_place_puts_the_bar_back() {
        let claude = Rect {
            left: 100,
            top: 50,
            right: 1300,
            bottom: 850,
        };
        for (left, top) in [(1100, 58), (120, 800), (1180, 790), (130, 60)] {
            let bar = Rect {
                left,
                top,
                right: left + 90,
                bottom: top + 30,
            };
            let anchor = Anchor::pin(bar, claude, 192);
            assert_eq!(anchor.place(90, 30, claude, 192), (left, top));
        }
    }

    #[test]
    fn missing_file_is_created_with_defaults() {
        let dir = TempDir::new("missing");
        let config = Config::load_from(&dir.file());
        assert_eq!(config, Config::default());
        let texts: Vec<_> = config.commands.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["/compact", "/clear"]);
        assert!(config.commands.iter().all(|c| c.mode == Mode::Send));
        assert!(dir.file().exists());
        assert_eq!(Config::load_from(&dir.file()), Config::default());
    }

    #[test]
    fn round_trip() {
        let dir = TempDir::new("round-trip");
        let mut config = Config::default();
        config.position.x = 42;
        config.commands[1].label = Some("Clear".into());
        config.commands.push(Command {
            text: "/compact keep the plan".into(),
            label: None,
            mode: Mode::Fill,
        });
        config.save_to(&dir.file()).unwrap();
        assert_eq!(Config::load_from(&dir.file()), config);
    }

    #[test]
    fn file_without_commands_gets_the_defaults_added() {
        let dir = TempDir::new("no-commands");
        std::fs::create_dir_all(&dir.0).unwrap();
        let json = r#"{"position": {"corner": "bottom_left", "x": 1, "y": 2}}"#;
        std::fs::write(dir.file(), json).unwrap();
        let config = Config::load_from(&dir.file());
        assert_eq!(config.position.corner, Corner::BottomLeft);
        assert_eq!(config.commands, Config::default().commands);
        let written = std::fs::read_to_string(dir.file()).unwrap();
        assert!(written.contains("/compact"));
    }

    #[test]
    fn bad_file_falls_back_and_is_left_alone() {
        let dir = TempDir::new("bad");
        std::fs::create_dir_all(&dir.0).unwrap();
        for json in [
            "{ not json",
            "[]",
            r#"{"commands": [{"command": "/x", "mode": "later"}]}"#,
        ] {
            std::fs::write(dir.file(), json).unwrap();
            let config = Config::load_from(&dir.file());
            assert!(config.is_unreadable());
            assert_eq!(config.commands, Config::default().commands);
            assert!(config.save_to(&dir.file()).is_err());
            assert_eq!(std::fs::read_to_string(dir.file()).unwrap(), json);
        }
    }
}
