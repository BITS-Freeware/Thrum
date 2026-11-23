//! Making peer- and server-chosen text safe to print.
//!
//! Message text, attachment names, ids and server answers reach the
//! terminal. A control character in them could move the cursor and
//! rewrite earlier lines, set the window title, add hidden links or write
//! the clipboard, and a bidirectional override could make one name look
//! like another. Everything the CLI prints passes through [`clean`].

/// `text` with every control character, and every character that changes
/// text direction, replaced by its visible escape (`\u{1b}`, `\n`), so a
/// printed line is exactly one line of the characters it shows.
pub fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() || is_direction_control(c) {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// Characters that override or isolate text direction, or mark it.
fn is_direction_control(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

/// `println!` with the formatted line passed through [`clean`].
#[macro_export]
macro_rules! outln {
    ($($arg:tt)*) => {
        println!("{}", $crate::display::clean(&format!($($arg)*)))
    };
}

/// `eprintln!` with the formatted line passed through [`clean`].
#[macro_export]
macro_rules! errln {
    ($($arg:tt)*) => {
        eprintln!("{}", $crate::display::clean(&format!($($arg)*)))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_controls_become_visible_text() {
        let line = clean("ok\u{1b}[2K\r\u{1b}[1Aalice: hi\u{1b}]52;c;eA==\u{7}\nnext\u{9b}");
        assert!(!line.chars().any(char::is_control), "{line:?}");
        assert_eq!(
            line,
            "ok\\u{1b}[2K\\r\\u{1b}[1Aalice: hi\\u{1b}]52;c;eA==\\u{7}\\nnext\\u{9b}"
        );
    }

    #[test]
    fn direction_overrides_are_escaped_and_ordinary_text_is_kept() {
        assert_eq!(clean("bob\u{202e}evila"), "bob\\u{202e}evila");
        assert_eq!(clean("héllo, 世界 👋"), "héllo, 世界 👋");
    }
}
