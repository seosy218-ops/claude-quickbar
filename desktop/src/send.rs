//! Types a phrase into Claude's prompt box, as if the user had typed it.
//! Everything that touches the system goes through [`Host`], so the steps
//! can be tested against a fake.

use serde::{Deserialize, Serialize};

/// How a phrase reaches the prompt box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Submit the phrase, then put the draft back.
    #[default]
    Send,
    /// Leave `phrase draft` in the box without submitting.
    Fill,
}

/// What one button types into Claude's prompt box: plain words such as `continue`,
/// or a Claude slash command with its arguments such as `/compact keep the plan`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phrase {
    /// The text typed, as is.
    #[serde(rename = "command")]
    pub text: String,
    /// Button label; the phrase text when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub mode: Mode,
}

impl Phrase {
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.text)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    ClaudeNotFound,
    /// Claude shows no prompt box of its Code page (settings, a chat), or the box
    /// would not take the focus. Nothing was typed.
    PromptNotFound,
}

/// The system as `send` sees it: Claude's window, its prompt box and the clipboard.
pub trait Host {
    /// Whatever it takes to put the clipboard back exactly as it was.
    type Clipboard;

    fn save_clipboard(&mut self) -> Self::Clipboard;
    fn restore_clipboard(&mut self, saved: Self::Clipboard);
    /// Brings Claude's window to the front.
    /// `false` when Claude is not running.
    fn activate_claude(&mut self) -> bool;
    /// Puts the focus in the prompt box of Claude's Code page, the one last used when
    /// the page is split. `false` when there is no such box or it would not take the focus.
    fn focus_prompt(&mut self) -> bool;
    /// Empties the prompt box and returns what was in it. May clobber the clipboard.
    fn cut_draft(&mut self) -> String;
    /// Inserts `text` at the caret. May clobber the clipboard.
    fn paste(&mut self, text: &str);
    /// Presses Enter in the prompt box.
    fn submit(&mut self);
}

pub fn send<H: Host>(host: &mut H, phrase: &Phrase) -> Result<(), SendError> {
    if !host.activate_claude() {
        return Err(SendError::ClaudeNotFound);
    }
    if !host.focus_prompt() {
        return Err(SendError::PromptNotFound);
    }
    let saved = host.save_clipboard();
    let draft = host.cut_draft();
    match phrase.mode {
        Mode::Send => {
            host.paste(&phrase.text);
            host.submit();
            if !draft.is_empty() {
                host.paste(&draft);
            }
        }
        Mode::Fill => host.paste(&format!("{} {}", phrase.text, draft)),
    }
    host.restore_clipboard(saved);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeHost {
        claude_open: bool,
        /// Claude shows the Code page's prompt box.
        prompt_shown: bool,
        /// The focus is in the prompt box; keys go elsewhere otherwise.
        focused: bool,
        input: String,
        clipboard: String,
        submitted: Vec<String>,
        /// Keys that landed outside the prompt box.
        stray_keys: usize,
    }

    impl FakeHost {
        fn new(input: &str) -> Self {
            FakeHost {
                claude_open: true,
                prompt_shown: true,
                focused: true,
                input: input.into(),
                clipboard: "user's clipboard".into(),
                submitted: Vec::new(),
                stray_keys: 0,
            }
        }

        /// Whether keys pressed now reach the prompt box; counts them when not.
        fn keys_reach_prompt(&mut self) -> bool {
            if !self.focused {
                self.stray_keys += 1;
            }
            self.focused
        }
    }

    impl Host for FakeHost {
        type Clipboard = String;

        fn save_clipboard(&mut self) -> String {
            self.clipboard.clone()
        }
        fn restore_clipboard(&mut self, saved: String) {
            self.clipboard = saved;
        }
        fn activate_claude(&mut self) -> bool {
            self.claude_open
        }
        fn focus_prompt(&mut self) -> bool {
            self.focused |= self.prompt_shown;
            self.prompt_shown
        }
        fn cut_draft(&mut self) -> String {
            if !self.keys_reach_prompt() {
                return String::new();
            }
            self.clipboard = std::mem::take(&mut self.input);
            self.clipboard.clone()
        }
        fn paste(&mut self, text: &str) {
            self.clipboard = text.into();
            if self.keys_reach_prompt() {
                self.input.push_str(text);
            }
        }
        fn submit(&mut self) {
            if self.keys_reach_prompt() {
                self.submitted.push(std::mem::take(&mut self.input));
            }
        }
    }

    fn phrase(text: &str, mode: Mode) -> Phrase {
        Phrase {
            text: text.into(),
            label: None,
            mode,
        }
    }

    #[test]
    fn send_without_draft() {
        let mut host = FakeHost::new("");
        send(&mut host, &phrase("/compact", Mode::Send)).unwrap();
        assert_eq!(host.submitted, ["/compact"]);
        assert_eq!(host.input, "");
        assert_eq!(host.clipboard, "user's clipboard");
    }

    #[test]
    fn send_puts_draft_back() {
        let mut host = FakeHost::new("写了一半的话");
        send(&mut host, &phrase("/compact", Mode::Send)).unwrap();
        assert_eq!(host.submitted, ["/compact"]);
        assert_eq!(host.input, "写了一半的话");
        assert_eq!(host.clipboard, "user's clipboard");
    }

    #[test]
    fn fill_puts_phrase_before_draft() {
        let mut host = FakeHost::new("keep the plan");
        send(&mut host, &phrase("/compact", Mode::Fill)).unwrap();
        assert!(host.submitted.is_empty());
        assert_eq!(host.input, "/compact keep the plan");
        assert_eq!(host.clipboard, "user's clipboard");
    }

    #[test]
    fn send_with_fixed_arguments() {
        let mut host = FakeHost::new("");
        send(&mut host, &phrase("/compact keep the plan", Mode::Send)).unwrap();
        assert_eq!(host.submitted, ["/compact keep the plan"]);
        assert_eq!(host.clipboard, "user's clipboard");
    }

    #[test]
    fn claude_not_open() {
        let mut host = FakeHost::new("draft");
        host.claude_open = false;
        let result = send(&mut host, &phrase("/compact", Mode::Send));
        assert_eq!(result, Err(SendError::ClaudeNotFound));
        assert!(host.submitted.is_empty());
        assert_eq!(host.input, "draft");
        assert_eq!(host.clipboard, "user's clipboard");
    }

    #[test]
    fn focus_elsewhere_goes_to_the_prompt_first() {
        let mut host = FakeHost::new("写了一半的话");
        host.focused = false;
        send(&mut host, &phrase("/compact", Mode::Send)).unwrap();
        assert_eq!(host.submitted, ["/compact"]);
        assert_eq!(host.input, "写了一半的话");
        assert_eq!(host.stray_keys, 0);
        assert_eq!(host.clipboard, "user's clipboard");
    }

    #[test]
    fn no_prompt_types_nothing() {
        let mut host = FakeHost::new("draft");
        host.prompt_shown = false;
        host.focused = false;
        let result = send(&mut host, &phrase("/compact", Mode::Send));
        assert_eq!(result, Err(SendError::PromptNotFound));
        assert!(host.submitted.is_empty());
        assert_eq!(host.stray_keys, 0);
        assert_eq!(host.input, "draft");
        assert_eq!(host.clipboard, "user's clipboard");
    }
}
