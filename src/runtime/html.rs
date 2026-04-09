//! HTML escaping for XSS prevention in FORGE template strings.

/// Escape HTML special characters: `&`, `<`, `>`, `"`, `'`.
pub fn html_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Generate a basic HTML document skeleton wrapping the given body.
pub fn html_layout(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n\
         <html>\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n\
         </head>\n\
         <body>\n\
         {}\n\
         </body>\n\
         </html>",
        html_escape(title),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_special_chars() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"hello\""), "&quot;hello&quot;");
        assert_eq!(html_escape("it's"), "it&#x27;s");
    }

    #[test]
    fn escape_passthrough() {
        assert_eq!(html_escape("hello world"), "hello world");
        assert_eq!(html_escape(""), "");
    }

    #[test]
    fn layout_produces_valid_html() {
        let result = html_layout("Test Page", "<h1>Hello</h1>");
        assert!(result.starts_with("<!DOCTYPE html>"));
        assert!(result.contains("<title>Test Page</title>"));
        assert!(result.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn layout_escapes_title() {
        let result = html_layout("<script>alert('xss')</script>", "body");
        assert!(result.contains("&lt;script&gt;"));
        assert!(!result.contains("<script>"));
    }
}
