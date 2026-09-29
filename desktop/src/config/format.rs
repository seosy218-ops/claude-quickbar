//! The bytes of `config.json` as users write them and as quickbar writes them.
//! Literal JSON only: a value that survives a round trip proves nothing about the file.

use super::*;

fn phrase(text: &str, label: Option<&str>, mode: Mode) -> Phrase {
    Phrase {
        text: text.into(),
        label: label.map(Into::into),
        mode,
    }
}

/// Every field, as `save` writes it.
const FULL: &str = r#"{
  "position": {
    "corner": "bottom_left",
    "x": -160,
    "y": 4
  },
  "commands": [
    {
      "command": "/compact",
      "mode": "send"
    },
    {
      "command": "/review",
      "label": "Review",
      "mode": "fill"
    }
  ]
}"#;

fn full() -> Config {
    Config {
        position: Anchor {
            corner: Corner::BottomLeft,
            x: -160,
            y: 4,
        },
        commands: vec![
            phrase("/compact", None, Mode::Send),
            phrase("/review", Some("Review"), Mode::Fill),
        ],
        unreadable: false,
    }
}

#[test]
fn full_file_reads() {
    assert_eq!(serde_json::from_str::<Config>(FULL).unwrap(), full());
}

#[test]
fn full_file_writes() {
    assert_eq!(serde_json::to_string_pretty(&full()).unwrap(), FULL);
}

#[test]
fn corners() {
    for (corner, json) in [
        (Corner::TopLeft, r#""top_left""#),
        (Corner::TopRight, r#""top_right""#),
        (Corner::BottomLeft, r#""bottom_left""#),
        (Corner::BottomRight, r#""bottom_right""#),
    ] {
        assert_eq!(serde_json::to_string(&corner).unwrap(), json);
        assert_eq!(serde_json::from_str::<Corner>(json).unwrap(), corner);
    }
}

/// Only `command` is required; everything else falls back.
#[test]
fn shortest_hand_written_file() {
    let json = r#"{"commands": [{"command": "/clear"}]}"#;
    let config = serde_json::from_str::<Config>(json).unwrap();
    assert_eq!(config.position, Anchor::default());
    assert_eq!(config.commands, [phrase("/clear", None, Mode::Send)]);
}

#[test]
fn default_position() {
    assert_eq!(
        serde_json::to_string(&Anchor::default()).unwrap(),
        r#"{"corner":"top_right","x":-160,"y":4}"#
    );
}
