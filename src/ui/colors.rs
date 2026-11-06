//! Application colors. RGB values are 0xRRGGBB; RGBA values are 0xRRGGBBAA.
//!
//! Keep theme choices here so views refer to a color's purpose, not its encoding.

use gitk_rs::cli::Theme;

pub(super) struct Palette {
    pub background: u32,
    pub foreground: u32,
    pub panel: u32,
    pub border: u32,
    pub dialog_button: u32,
    pub row: u32,
    pub selection: u32,
    pub local_foreground: u32,
    pub staged: u32,
    pub unstaged: u32,
    pub added_text: u32,
    pub removed_text: u32,
    pub added_line: u32,
    pub removed_line: u32,
    pub hunk_line: u32,
    pub header_line: u32,
}

const DARK: Palette = Palette {
    background: 0x202329,
    foreground: 0xe0e4eb,
    panel: 0x292d35,
    border: 0x454b57,
    dialog_button: 0x454b57,
    row: 0x252930,
    selection: 0x285b8e,
    local_foreground: 0xffffff,
    staged: 0x1f3b2a,
    unstaged: 0x4a1f23,
    added_text: 0x6ddf6d,
    removed_text: 0xff7b72,
    added_line: 0x184b32,
    removed_line: 0x5f2639,
    hunk_line: 0x26384c,
    header_line: 0x343a44,
};

const LIGHT: Palette = Palette {
    background: 0xf5f6f8,
    foreground: 0x202329,
    panel: 0xffffff,
    border: 0xc4c9d2,
    dialog_button: 0xdde2e8,
    row: 0xffffff,
    selection: 0xcfe7ff,
    local_foreground: 0x202329,
    staged: 0xe2f7e1,
    unstaged: 0xfde2e1,
    added_text: 0x1f7a1f,
    removed_text: 0xb42318,
    added_line: 0xd9f5e0,
    removed_line: 0xffdddd,
    hunk_line: 0xddeafa,
    header_line: 0xe6e9ed,
};

impl Palette {
    pub const fn for_theme(theme: Theme) -> &'static Self {
        Self::for_dark(matches!(theme, Theme::Dark))
    }

    pub const fn for_dark(dark: bool) -> &'static Self {
        if dark { &DARK } else { &LIGHT }
    }
}

// Commit and branch selections use a solid highlight in both themes.
pub(super) const SOLID_SELECTION: u32 = 0x285b8e;
pub(super) const ON_SOLID: u32 = 0xffffff;
pub(super) const HEAD_BADGE: u32 = 0x735a12;
pub(super) const BRANCH_BADGE: u32 = 0x24643a;
pub(super) const REMOTE_BADGE: u32 = 0x2e527c;
pub(super) const TAG_BADGE: u32 = 0x654b7e;
pub(super) const GRAPH_LANES: [u32; 6] =
    [0x80bfff, 0x83d49b, 0xe5b978, 0xd49fe8, 0x73d5d1, 0xef9090];
pub(super) const GRAPH_FOLDED: u32 = 0x9298a3;
pub(super) const GRAPH_OVERFLOW: u32 = 0xe5b978;
pub(super) const DIALOG_OVERLAY: u32 = 0x00000099;
pub(super) const DIFF_TEXT_SELECTION: u32 = 0x4285f4aa;
