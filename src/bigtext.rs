use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    widgets::Widget,
};

/// Simple big text widget using block characters
pub struct BigText {
    text: String,
    style: Style,
}

impl BigText {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    fn char_to_blocks(c: char) -> &'static [&'static str] {
        match c.to_ascii_uppercase() {
            'A' => &[
                " ███ ",
                "█   █",
                "█   █",
                "█████",
                "█   █",
            ],
            'B' => &[
                "████ ",
                "█   █",
                "████ ",
                "█   █",
                "████ ",
            ],
            'C' => &[
                " ████",
                "█    ",
                "█    ",
                "█    ",
                " ████",
            ],
            'D' => &[
                "████ ",
                "█   █",
                "█   █",
                "█   █",
                "████ ",
            ],
            'E' => &[
                "█████",
                "█    ",
                "████ ",
                "█    ",
                "█████",
            ],
            'F' => &[
                "█████",
                "█    ",
                "████ ",
                "█    ",
                "█    ",
            ],
            'G' => &[
                " ████",
                "█    ",
                "█  ██",
                "█   █",
                " ████",
            ],
            'H' => &[
                "█   █",
                "█   █",
                "█████",
                "█   █",
                "█   █",
            ],
            'I' => &[
                "███",
                " █ ",
                " █ ",
                " █ ",
                "███",
            ],
            'J' => &[
                "  ███",
                "    █",
                "    █",
                "█   █",
                " ███ ",
            ],
            'K' => &[
                "█   █",
                "█  █ ",
                "███  ",
                "█  █ ",
                "█   █",
            ],
            'L' => &[
                "█    ",
                "█    ",
                "█    ",
                "█    ",
                "█████",
            ],
            'M' => &[
                "█   █",
                "██ ██",
                "█ █ █",
                "█   █",
                "█   █",
            ],
            'N' => &[
                "█   █",
                "██  █",
                "█ █ █",
                "█  ██",
                "█   █",
            ],
            'O' => &[
                " ███ ",
                "█   █",
                "█   █",
                "█   █",
                " ███ ",
            ],
            'P' => &[
                "████ ",
                "█   █",
                "████ ",
                "█    ",
                "█    ",
            ],
            'Q' => &[
                " ███ ",
                "█   █",
                "█   █",
                "█ █ █",
                " ██ █",
            ],
            'R' => &[
                "████ ",
                "█   █",
                "████ ",
                "█  █ ",
                "█   █",
            ],
            'S' => &[
                " ████",
                "█    ",
                " ███ ",
                "    █",
                "████ ",
            ],
            'T' => &[
                "█████",
                "  █  ",
                "  █  ",
                "  █  ",
                "  █  ",
            ],
            'U' => &[
                "█   █",
                "█   █",
                "█   █",
                "█   █",
                " ███ ",
            ],
            'V' => &[
                "█   █",
                "█   █",
                "█   █",
                " █ █ ",
                "  █  ",
            ],
            'W' => &[
                "█   █",
                "█   █",
                "█ █ █",
                "█ █ █",
                " █ █ ",
            ],
            'X' => &[
                "█   █",
                " █ █ ",
                "  █  ",
                " █ █ ",
                "█   █",
            ],
            'Y' => &[
                "█   █",
                " █ █ ",
                "  █  ",
                "  █  ",
                "  █  ",
            ],
            'Z' => &[
                "█████",
                "   █ ",
                "  █  ",
                " █   ",
                "█████",
            ],
            '0' => &[
                " ███ ",
                "█  ██",
                "█ █ █",
                "██  █",
                " ███ ",
            ],
            '1' => &[
                " ██ ",
                "█ █ ",
                "  █ ",
                "  █ ",
                "████",
            ],
            '2' => &[
                "████ ",
                "    █",
                " ███ ",
                "█    ",
                "█████",
            ],
            '3' => &[
                "████ ",
                "    █",
                " ███ ",
                "    █",
                "████ ",
            ],
            '4' => &[
                "█   █",
                "█   █",
                "█████",
                "    █",
                "    █",
            ],
            '5' => &[
                "█████",
                "█    ",
                "████ ",
                "    █",
                "████ ",
            ],
            '6' => &[
                " ███ ",
                "█    ",
                "████ ",
                "█   █",
                " ███ ",
            ],
            '7' => &[
                "█████",
                "   █ ",
                "  █  ",
                " █   ",
                "█    ",
            ],
            '8' => &[
                " ███ ",
                "█   █",
                " ███ ",
                "█   █",
                " ███ ",
            ],
            '9' => &[
                " ███ ",
                "█   █",
                " ████",
                "    █",
                " ███ ",
            ],
            ' ' => &[
                "  ",
                "  ",
                "  ",
                "  ",
                "  ",
            ],
            _ => &[
                "█████",
                "█   █",
                "█   █",
                "█   █",
                "█████",
            ],
        }
    }
}

impl Widget for BigText {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chars: Vec<char> = self.text.chars().collect();
        if chars.is_empty() {
            return;
        }

        let height = 5; // Each character is 5 lines tall
        let char_width = 6; // Width per character including spacing

        // Calculate starting position to center the text
        let total_width = chars.len() * char_width;
        let start_x = area.x + (area.width.saturating_sub(total_width as u16)) / 2;
        let start_y = area.y;

        for (char_idx, &ch) in chars.iter().enumerate() {
            let lines = Self::char_to_blocks(ch);
            let x_offset = start_x + (char_idx * char_width) as u16;

            for (line_idx, line) in lines.iter().enumerate() {
                let y = start_y + line_idx as u16;
                if y >= area.y + area.height || y >= buf.area.height {
                    break;
                }

                for (col_idx, c) in line.chars().enumerate() {
                    let x = x_offset + col_idx as u16;
                    if x >= area.x + area.width || x >= buf.area.width {
                        break;
                    }

                    if c != ' ' {
                        buf.get_mut(x, y)
                            .set_char(c)
                            .set_style(self.style);
                    }
                }
            }
        }
    }
}
