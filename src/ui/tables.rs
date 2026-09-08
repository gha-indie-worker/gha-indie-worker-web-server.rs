#![forbid(unsafe_code)]

//! One table shape, reused by runs, workers, plans, members, seats and audit.
//!
//! The wrapper scrolls horizontally on its own (`.table-wrap`) so a wide table
//! never makes the page scroll sideways — which matters most on `m.`, where the
//! same tables are rendered in the compact layout.

use maud::{html, Markup};

/// A column definition.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub label: &'static str,
    /// Right-aligned monospace, for counts and durations.
    pub numeric: bool,
}

impl Column {
    #[must_use]
    pub const fn text(label: &'static str) -> Self {
        Self { label, numeric: false }
    }

    #[must_use]
    pub const fn numeric(label: &'static str) -> Self {
        Self { label, numeric: true }
    }
}

/// Renders a table. `rows` are pre-rendered `<td>` sequences, so a caller can
/// put a badge, a link or a form inside a cell without this module knowing.
#[must_use]
pub fn table(caption: &str, columns: &[Column], rows: Vec<Markup>) -> Markup {
    html! {
        div class="table-wrap" {
            table {
                @if !caption.is_empty() {
                    caption class="label" { (caption) }
                }
                thead {
                    tr {
                        @for column in columns {
                            @if column.numeric {
                                th scope="col" class="numeric" { (column.label) }
                            } @else {
                                th scope="col" { (column.label) }
                            }
                        }
                    }
                }
                tbody {
                    @for row in &rows {
                        tr { (row) }
                    }
                }
            }
        }
        @if rows.is_empty() {
            div class="empty" { p { "Nothing to show yet." } }
        }
    }
}

/// A plain text cell.
#[must_use]
pub fn cell(value: &str) -> Markup {
    html! { td { (value) } }
}

/// A right-aligned monospace cell.
#[must_use]
pub fn numeric_cell(value: &str) -> Markup {
    html! { td class="numeric" { (value) } }
}

/// A cell whose content is a link.
#[must_use]
pub fn link_cell(href: &str, label: &str) -> Markup {
    html! { td { a href=(href) { (label) } } }
}

/// A cell holding arbitrary pre-rendered markup (a badge, a small form).
#[must_use]
pub fn markup_cell(inner: Markup) -> Markup {
    html! { td { (inner) } }
}

/// Joins cells into one row payload.
#[must_use]
pub fn row(cells: Vec<Markup>) -> Markup {
    html! {
        @for cell in &cells {
            (cell)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_renders_a_scroll_container_and_column_scopes() {
        let markup = table(
            "Runs",
            &[Column::text("Run"), Column::numeric("Duration")],
            vec![row(vec![cell("run_1"), numeric_cell("42s")])],
        )
        .into_string();
        assert!(markup.contains(r#"<div class="table-wrap">"#));
        assert!(markup.contains(r#"<th scope="col">Run</th>"#));
        assert!(markup.contains(r#"<th scope="col" class="numeric">Duration</th>"#));
        assert!(markup.contains(r#"<td class="numeric">42s</td>"#));
    }

    #[test]
    fn an_empty_table_shows_an_empty_state_instead_of_a_bare_header() {
        let markup = table("Runs", &[Column::text("Run")], vec![]).into_string();
        assert!(markup.contains("Nothing to show yet."));
    }

    #[test]
    fn cell_content_is_escaped() {
        let markup = cell("<b>bold</b>").into_string();
        assert_eq!(markup, "<td>&lt;b&gt;bold&lt;/b&gt;</td>");
        assert!(link_cell("/runs/1", "<i>x</i>").into_string().contains("&lt;i&gt;"));
    }

    #[test]
    fn markup_cells_pass_pre_rendered_content_through() {
        let markup = markup_cell(crate::ui::components::status_badge("succeeded")).into_string();
        assert!(markup.contains(r#"<span class="badge ok">succeeded</span>"#));
    }
}
