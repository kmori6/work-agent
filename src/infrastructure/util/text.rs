/// Truncate a string to at most `max_chars` characters.
///
/// Returns the (possibly truncated) string and whether truncation occurred.
pub fn truncate_text(text: String, max_chars: usize) -> (String, bool) {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return (text, false);
    }

    let truncated: String = text.chars().take(max_chars).collect();
    (format!("{truncated}\n\n... [truncated] ..."), true)
}
