//! Minimal logging to stderr, one line per message.
//!
//! Messages routinely contain names and error strings that originate from the
//! Docker API. Line breaks are escaped so that none of them can forge an
//! additional log line.

use std::fmt::Display;

pub fn info(message: impl Display) {
    eprintln!("{}", format_line("INFO ", &message.to_string()));
}

pub fn warn(message: impl Display) {
    eprintln!("{}", format_line("WARN ", &message.to_string()));
}

pub fn error(message: impl Display) {
    eprintln!("{}", format_line("ERROR", &message.to_string()));
}

fn format_line(level: &str, message: &str) -> String {
    let escaped = message.replace('\r', "\\r").replace('\n', "\\n");
    format!("{level} {escaped}")
}

#[cfg(test)]
mod tests {
    use super::format_line;

    #[test]
    fn a_message_stays_on_one_line() {
        let line = format_line("INFO ", "first\nERROR forged\r\nthird");
        assert_eq!(line, "INFO  first\\nERROR forged\\r\\nthird");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn the_level_prefix_is_kept() {
        assert!(format_line("WARN ", "x").starts_with("WARN "));
    }
}
