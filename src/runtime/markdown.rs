//! Markdown rendering for FORGE — converts Markdown to HTML via pulldown-cmark.

use pulldown_cmark::{html, Options, Parser};

/// Render Markdown text to HTML.
///
/// Enables tables, footnotes, strikethrough, and task lists.
/// Fenced code blocks tagged `forge` get `class="language-forge"` for Prism.js.
pub fn render_markdown(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(input, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(render_markdown(""), "");
    }

    #[test]
    fn renders_heading() {
        let result = render_markdown("# Hello");
        assert!(result.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn renders_paragraph() {
        let result = render_markdown("Hello world");
        assert!(result.contains("<p>Hello world</p>"));
    }

    #[test]
    fn forge_code_block_gets_language_class() {
        let input = "```forge\ntask greet(name: Text) -> Text\n```";
        let result = render_markdown(input);
        assert!(
            result.contains("language-forge"),
            "expected language-forge class, got: {}",
            result
        );
    }

    #[test]
    fn renders_table() {
        let input = "| A | B |\n|---|---|\n| 1 | 2 |";
        let result = render_markdown(input);
        assert!(result.contains("<table>"));
        assert!(result.contains("<td>1</td>"));
    }

    #[test]
    fn renders_strikethrough() {
        let result = render_markdown("~~deleted~~");
        assert!(result.contains("<del>deleted</del>"));
    }

    #[test]
    fn renders_task_list() {
        let input = "- [x] done\n- [ ] todo";
        let result = render_markdown(input);
        assert!(result.contains("checked"));
        assert!(result.contains("type=\"checkbox\""));
    }

    #[test]
    fn renders_links() {
        let result = render_markdown("[click](https://example.com)");
        assert!(result.contains("<a href=\"https://example.com\">click</a>"));
    }

    #[test]
    fn invalid_markdown_passes_through() {
        let input = "some ][ broken [[ markup";
        let result = render_markdown(input);
        assert!(result.contains("some ][ broken [[ markup"));
    }
}
