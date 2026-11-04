//! Text positions refer to original UTF-8 bytes, including diff markers. Display
//! tabs may expand to spaces without changing clipboard contents or offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPoint {
    pub line: usize,
    pub byte: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct TextSelection {
    pub anchor: TextPoint,
    pub cursor: TextPoint,
}

impl TextSelection {
    pub fn ordered(self) -> (TextPoint, TextPoint) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    pub fn on_line(self, line: usize, length: usize) -> Option<std::ops::Range<usize>> {
        let (start, end) = self.ordered();
        if line < start.line || line > end.line {
            return None;
        }
        let from = if line == start.line {
            start.byte.min(length)
        } else {
            0
        };
        let to = if line == end.line {
            end.byte.min(length)
        } else {
            length
        };
        (to > from).then_some(from..to)
    }
}

#[derive(Clone, Debug)]
pub struct DisplayText {
    pub text: String,
    boundaries: Vec<(usize, usize)>,
}

impl DisplayText {
    pub fn new(source: &str) -> Self {
        let mut text = String::new();
        let mut boundaries = vec![(0, 0)];
        let mut column = 0;
        for (byte, character) in source.char_indices() {
            if character == '\t' {
                let spaces = 4 - column % 4;
                text.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            } else {
                text.push(character);
                column += 1;
            }
            boundaries.push((byte + character.len_utf8(), text.len()));
        }
        Self { text, boundaries }
    }

    pub fn display_index(&self, source: usize) -> usize {
        let index = self.boundaries.partition_point(|(byte, _)| *byte < source);
        self.boundaries
            .get(index)
            .or_else(|| self.boundaries.last())
            .unwrap()
            .1
    }

    pub fn source_index(&self, display: usize) -> usize {
        let index = self.boundaries.partition_point(|(_, byte)| *byte < display);
        let after = self
            .boundaries
            .get(index)
            .or_else(|| self.boundaries.last())
            .unwrap();
        let before = self.boundaries.get(index.saturating_sub(1)).unwrap();
        if display.saturating_sub(before.1) < after.1.saturating_sub(display) {
            before.0
        } else {
            after.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tab_expansion_preserves_original_unicode_byte_offsets() {
        let display = DisplayText::new("\tλx");
        assert_eq!(display.text, "    λx");
        assert_eq!(display.display_index(1), 4);
        assert_eq!(display.display_index(3), 6);
        assert_eq!(display.source_index(1), 0);
        assert_eq!(display.source_index(3), 1);
        assert_eq!(display.source_index(6), 3);
    }
    #[test]
    fn reversed_cross_line_selections_are_normalized() {
        let selection = TextSelection {
            anchor: TextPoint { line: 10, byte: 5 },
            cursor: TextPoint { line: 2, byte: 3 },
        };
        assert_eq!(selection.on_line(2, 20), Some(3..20));
        assert_eq!(selection.on_line(7, 20), Some(0..20));
        assert_eq!(selection.on_line(10, 20), Some(0..5));
        assert_eq!(selection.on_line(11, 20), None);
    }
}
