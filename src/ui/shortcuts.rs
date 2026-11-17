//! Application bindings and Help rows share keystrokes and platform aliases.
use super::*;
use gitk_rs::search::Qualifier;
use gpui_kit::component::input;

#[derive(Clone, Copy)]
struct Key {
    stroke: &'static str,
    command: Option<&'static str>,
}

const fn key(stroke: &'static str) -> Key {
    Key {
        stroke,
        command: None,
    }
}

// Only application shortcuts gain Command aliases; Control editing stays literal.
macro_rules! command {
    ($suffix:literal) => {
        Key {
            stroke: concat!("ctrl-", $suffix),
            command: Some(concat!("cmd-", $suffix)),
        }
    };
}

impl Key {
    fn label(self, macos: bool) -> String {
        let stroke = if macos {
            self.command.unwrap_or(self.stroke)
        } else {
            self.stroke
        };
        if stroke == "g g" {
            return "gg".into();
        }
        if let Some(letter) = stroke.strip_prefix("shift-")
            && letter.len() == 1
            && letter.as_bytes()[0].is_ascii_alphabetic()
        {
            return letter.to_ascii_uppercase();
        }
        let modified = stroke.contains('-');
        stroke
            .split('-')
            .map(|part| match part {
                "ctrl" => "Ctrl".into(),
                "cmd" => "Cmd".into(),
                "alt" => if macos { "Option" } else { "Alt" }.into(),
                "shift" => "Shift".into(),
                "enter" => if macos { "Return" } else { "Enter" }.into(),
                "pageup" => "PageUp".into(),
                "pagedown" => "PageDown".into(),
                "backspace" => "Backspace".into(),
                "delete" => "Delete".into(),
                "escape" => "Escape".into(),
                "space" => "Space".into(),
                "home" => "Home".into(),
                "end" => "End".into(),
                "up" => "Up".into(),
                "down" => "Down".into(),
                "f1" => "F1".into(),
                "f5" => "F5".into(),
                _ if modified => part.to_ascii_uppercase(),
                _ => part.into(),
            })
            .collect::<Vec<String>>()
            .join("+")
    }
}

pub(super) struct Shortcut {
    keys: &'static [Key],
    hint: Option<&'static str>,
    pub(super) description: &'static str,
    bindings: Option<fn(bool) -> Vec<KeyBinding>>,
}

impl Shortcut {
    const fn hint(hint: &'static str, description: &'static str) -> Self {
        Self {
            keys: &[],
            hint: Some(hint),
            description,
            bindings: None,
        }
    }

    pub(super) fn label(&self) -> String {
        self.label_for(cfg!(target_os = "macos"))
    }

    fn label_for(&self, macos: bool) -> String {
        self.hint.map_or_else(
            || {
                self.keys
                    .iter()
                    .map(|key| key.label(macos))
                    .collect::<Vec<_>>()
                    .join(" / ")
            },
            str::to_owned,
        )
    }
}

// Each action may have several keys and focus contexts; Help lists each key once.
macro_rules! shortcut {
    ($description:literal; $($contexts:expr, $action:ty => [$($key:expr),+ $(,)?]);+ $(;)?) => {
        Shortcut {
            keys: &[$($($key),+),+],
            hint: None,
            description: $description,
            bindings: Some(|macos| {
                let mut bindings = Vec::new();
                $(for context in $contexts {
                    for key in [$($key),+] {
                        bindings.push(KeyBinding::new(key.stroke, <$action>::default(), context));
                        if macos && let Some(command) = key.command {
                            bindings.push(KeyBinding::new(command, <$action>::default(), context));
                        }
                    }
                })+
                bindings
            }),
        }
    };
    ($description:literal; [$($key:expr),+ $(,)?]) => {
        Shortcut { keys: &[$($key),+], hint: None, description: $description, bindings: None }
    };
}

const HISTORY: Option<&str> = Some("GitHistory && !Input && !Dialog");
const HISTORY_INPUT: Option<&str> = Some("GitHistory && !Dialog");
const INPUT: Option<&str> = Some("Input");
const DIFF: Option<&str> = Some("Diff");
const BRANCH: Option<&str> = Some("BranchDialog");
const BRANCH_INPUT: Option<&str> = Some("BranchDialog > Input");
const HELP: Option<&str> = Some("HelpDialog");

pub(super) const REGEX: Shortcut = shortcut!("Toggle regex search";
    [HISTORY_INPUT], ToggleRegex => [key("alt-r")]);

pub(super) struct ShortcutGroup {
    pub(super) title: &'static str,
    pub(super) shortcuts: &'static [Shortcut],
    pub(super) qualifiers: &'static [Qualifier],
}

pub(super) const GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title: "Commit list",
        qualifiers: &[],
        shortcuts: &[
            shortcut!("Move up one commit"; [HISTORY], Previous => [key("up"), key("p"), key("k")]),
            shortcut!("Move down one commit"; [HISTORY], Next => [key("down"), key("n"), key("j")]),
            shortcut!("Select next / previous commit";
                [HISTORY_INPUT], Next => [key("alt-j")];
                [HISTORY_INPUT], Previous => [key("alt-k")]),
            shortcut!("Jump to the first commit"; [HISTORY], First => [key("home"), key("g g")]),
            shortcut!("Jump to the last loaded commit"; [HISTORY], Last => [key("end"), key("shift-g")]),
            shortcut!("Scroll history up / down a page";
                [HISTORY], HistoryPageUp => [command!("pageup")];
                [HISTORY], HistoryPageDown => [command!("pagedown")]),
            shortcut!("Copy commit hash";
                [Some("GitHistory && !Input && !Diff && !Dialog")], CopyCommit => [command!("c")]),
        ],
    },
    ShortcutGroup {
        title: "Diff view",
        qualifiers: &[],
        shortcuts: &[
            shortcut!("Select previous / next blame line";
                [Some("Blame && !Input")], diff_view::BlameUp => [key("up")];
                [Some("Blame && !Input")], diff_view::BlameDown => [key("down")]),
            shortcut!("Open selected blame commit"; [Some("Blame && !Input")], diff_view::OpenBlameCommit => [key("enter")]),
            shortcut!("Scroll diff up one page"; [HISTORY], diff_view::PageUp => [key("delete"), key("backspace"), key("b"), key("shift-b")]),
            shortcut!("Scroll diff down one page"; [HISTORY], diff_view::PageDown => [key("space")]),
            shortcut!("Scroll diff up 18 lines"; [HISTORY], diff_view::LinesUp => [key("u"), key("shift-u")]),
            shortcut!("Scroll diff down 18 lines"; [HISTORY], diff_view::LinesDown => [key("d"), key("shift-d")]),
            Shortcut::hint("Drag / Shift-click", "Select text / extend selection"),
            Shortcut::hint("Double-click", "Select a word"),
            shortcut!("Select all diff text"; [DIFF], diff_view::SelectAll => [command!("a")]),
            shortcut!("Copy selected text"; [DIFF], diff_view::Copy => [command!("c")]),
            shortcut!("Copy without +/- markers"; [DIFF], diff_view::CopyCode => [command!("shift-c")]),
        ],
    },
    ShortcutGroup {
        title: "General",
        qualifiers: &[],
        shortcuts: &[
            shortcut!("Focus the filter box"; [HISTORY], FocusFilter => [key("/")]),
            shortcut!("Apply filter and return to history (in filter box)"; [key("enter")]),
            shortcut!("Dismiss filter suggestions, then leave the filter box";
                [Some("Input && !Dialog"), HISTORY_INPUT], BlurFilter => [key("escape")]),
            shortcut!("Reload commits"; [None], Reload => [key("f5")]),
            REGEX,
            shortcut!("Show help (? outside text inputs)";
                [HISTORY_INPUT], Help => [key("f1")];
                [HISTORY], Help => [key("?")]),
            shortcut!("Open repository"; [HISTORY_INPUT], OpenRepository => [command!("o")]),
            shortcut!("Open repository by path"; [HISTORY_INPUT], OpenRepositoryPath => [command!("shift-o")]),
            shortcut!("Switch local branch (outside text inputs)"; [HISTORY], OpenBranches => [command!("b")]),
            shortcut!("Open preferences"; [HISTORY_INPUT], OpenPreferences => [command!(",")]),
            shortcut!("Toggle theme (outside text inputs)"; [HISTORY], ToggleTheme => [key("alt-t")]),
            shortcut!("Quit gitk-rs"; [None], Quit => [command!("q")]),
        ],
    },
    ShortcutGroup {
        title: "Filter box and text inputs",
        qualifiers: &[],
        shortcuts: &[
            Shortcut::hint(
                "Tab / Up / Down",
                "Complete filter tag / choose suggestion (when shown)",
            ),
            shortcut!("Move to start / end of line";
                [INPUT], input::MoveHome => [key("ctrl-a")];
                [INPUT], input::MoveEnd => [key("ctrl-e")]),
            shortcut!("Move back / forward one character";
                [INPUT], input::MoveLeft => [key("ctrl-b")];
                [INPUT], input::MoveRight => [key("ctrl-f")]),
            shortcut!("Delete word before cursor"; [INPUT], input::DeleteToPreviousWordStart => [key("ctrl-w")]),
            shortcut!("Delete to start of line"; [INPUT], input::DeleteToBeginningOfLine => [key("ctrl-u")]),
            shortcut!("Delete to end of line"; [INPUT], input::DeleteToEndOfLine => [key("ctrl-k")]),
            shortcut!("Delete character at cursor"; [INPUT], input::Delete => [key("ctrl-d")]),
        ],
    },
    ShortcutGroup {
        title: "Branch dialog",
        qualifiers: &[],
        shortcuts: &[
            shortcut!("Select previous branch";
                [BRANCH, BRANCH_INPUT], BranchUp => [key("up")];
                [BRANCH_INPUT], BranchUp => [key("ctrl-p"), key("alt-k")]),
            shortcut!("Select next branch";
                [BRANCH, BRANCH_INPUT], BranchDown => [key("down")];
                [BRANCH_INPUT], BranchDown => [key("ctrl-n"), key("alt-j")]),
            shortcut!("Switch to selected branch"; [BRANCH], BranchAccept => [key("enter")]),
            Shortcut::hint("Type in filter", "Filter local branches"),
        ],
    },
    ShortcutGroup {
        title: "Help dialog",
        qualifiers: &[],
        shortcuts: &[
            shortcut!("Scroll up / down one line";
                [HELP], HelpScrollUp => [key("up")];
                [HELP], HelpScrollDown => [key("down")]),
            shortcut!("Scroll up / down one page";
                [HELP], HelpPageUp => [key("pageup")];
                [HELP], HelpPageDown => [key("pagedown")]),
            shortcut!("Jump to the top / bottom";
                [HELP], HelpTop => [key("home")];
                [HELP], HelpBottom => [key("end")]),
        ],
    },
    ShortcutGroup {
        title: "Dialogs",
        qualifiers: &[],
        shortcuts: &[
            shortcut!("Close the dialog";
                [Some("Dialog"), Some("Dialog > Input && !Select")], CloseDialog => [key("escape")]),
            shortcut!("Open repository in the path input"; [key("enter")]),
            shortcut!("Apply and save preferences"; [Some("Dialog")], ApplyPreferences => [command!("enter")]),
        ],
    },
    ShortcutGroup {
        title: "Commit search",
        shortcuts: &[],
        qualifiers: &Qualifier::ALL,
    },
];

const FORCE_RELOAD: Shortcut = shortcut!(""; [None], ForceReload => [key("shift-f5")]);

fn bindings(macos: bool) -> Vec<KeyBinding> {
    GROUPS
        .iter()
        .flat_map(|group| group.shortcuts)
        .chain([&FORCE_RELOAD])
        .filter_map(|shortcut| shortcut.bindings)
        .flat_map(|bindings| bindings(macos))
        .collect()
}

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys(bindings(cfg!(target_os = "macos")));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_labels_preserve_literal_control_editing_and_uppercase_keys() {
        for macos in [false, true] {
            assert_eq!(key("ctrl-a").label(macos), "Ctrl+A");
            assert_eq!(key("shift-g").label(macos), "G");
            assert_eq!(key("g g").label(macos), "gg");
            assert_eq!(
                command!("shift-c").label(macos),
                if macos { "Cmd+Shift+C" } else { "Ctrl+Shift+C" }
            );
            assert_eq!(
                REGEX.label_for(macos),
                if macos { "Option+R" } else { "Alt+R" }
            );
            assert_eq!(
                key("enter").label(macos),
                if macos { "Return" } else { "Enter" }
            );
        }
    }

    #[test]
    fn command_aliases_are_limited_to_application_shortcuts() {
        let normal = bindings(false);
        let mac = bindings(true);
        assert!(mac.len() > normal.len());
        for binding in &mac {
            if binding.action().partial_eq(&input::MoveHome) {
                assert_eq!(binding.keystrokes()[0].key(), "a");
                assert!(binding.keystrokes()[0].modifiers().control);
                assert!(!binding.keystrokes()[0].modifiers().platform);
            }
        }
    }
}
