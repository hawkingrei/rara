use std::path::Path;

use ratatui::text::Line;

pub(crate) fn append_markdown(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
    lines: &mut Vec<Line<'static>>,
) {
    let rendered = crate::tui::markdown_render::render_markdown_text_with_width_and_cwd(
        markdown_source,
        width,
        cwd,
    );
    crate::tui::line_utils::push_owned_lines(&rendered.lines, lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_to_strings(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.clone())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn append_markdown_preserves_plain_text_line() {
        let mut out = Vec::new();
        append_markdown(
            "Hi! How can I help with rara today?\n",
            None,
            None,
            &mut out,
        );
        assert_eq!(
            lines_to_strings(&out),
            vec!["Hi! How can I help with rara today?".to_string()]
        );
    }

    #[test]
    fn append_markdown_renders_ordered_list_item_on_one_line() {
        let mut out = Vec::new();
        append_markdown("1. Tight item\n", None, None, &mut out);
        assert_eq!(lines_to_strings(&out), vec!["1. Tight item".to_string()]);
    }

    #[test]
    fn append_markdown_renders_tables_with_stable_columns() {
        let mut out = Vec::new();
        append_markdown(
            concat!(
                "| Risk | Mitigation |\n",
                "| --- | --- |\n",
                "| assembler.rs split | context tests |\n",
                "| OpenAI embedding API cost | hash fallback |\n",
            ),
            Some(80),
            None,
            &mut out,
        );

        let rendered = lines_to_strings(&out);
        let pipe_columns = rendered
            .iter()
            .filter_map(|line| line.find('|'))
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("Risk")));
        assert!(rendered
            .iter()
            .any(|line| line.contains("OpenAI embedding API cost")));
        assert!(!pipe_columns.is_empty());
        assert!(pipe_columns.iter().all(|column| *column == pipe_columns[0]));
    }
}
