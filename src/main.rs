use std::cell::RefCell;
use std::path::{Path, PathBuf};

use viewkit::draw_command::DrawCommand;
use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

struct EditApp {
    editor: TextEditorInteractionState,
    document: DocumentIdentity,
}

impl App for EditApp {
    type Body = EditView;

    fn new() -> Self {
        let document = DocumentIdentity::from_arguments();
        let editor = TextEditorInteractionState::new();
        if let Some(contents) = document.contents.as_ref() {
            editor.set_value(contents.clone());
        }
        Self { editor, document }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new(format!("{} — Edit", self.document.display_name))
            .size(860.0, 640.0)
            .resizable(true)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        EditView::new(
            self.editor.clone(),
            self.document.file_type,
            self.document.encoding,
        )
    }
}

#[derive(Clone)]
struct DocumentIdentity {
    display_name: String,
    file_type: &'static str,
    encoding: &'static str,
    contents: Option<String>,
}

impl DocumentIdentity {
    fn from_arguments() -> Self {
        let path = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .find(|argument| {
                let value = argument.to_string_lossy();
                !value.is_empty()
                    && !value.starts_with("--")
                    && !value.starts_with("__MNU_")
                    && !value.bytes().all(|byte| byte.is_ascii_digit())
            });
        let Some(path) = path else {
            return Self {
                display_name: "Untitled".into(),
                file_type: "Plain Text",
                encoding: "UTF-8",
                contents: None,
            };
        };

        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .to_owned();
        let file_type = file_type_for_path(&path);
        let (contents, encoding) = match std::fs::read(&path) {
            Ok(bytes) if bytes.starts_with(&[0xef, 0xbb, 0xbf]) => (
                String::from_utf8(bytes[3..].to_vec()).ok(),
                "UTF-8 with BOM",
            ),
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(contents) => (Some(contents), "UTF-8"),
                Err(_) => (None, "Unsupported encoding"),
            },
            Err(_) => (None, "UTF-8"),
        };
        Self {
            display_name,
            file_type,
            encoding,
            contents,
        }
    }
}

fn file_type_for_path(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "md" | "markdown" => "Markdown",
        "xml" => "XML",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" | "mts" | "cts" => "TypeScript",
        "rs" => "Rust",
        "c" | "h" => "C",
        "cc" | "cpp" | "cxx" | "hpp" => "C++",
        "py" => "Python",
        "sh" | "bash" | "zsh" => "Shell Script",
        _ => "Plain Text",
    }
}

#[derive(Default)]
struct FindState {
    visible: bool,
    last_query: String,
    result: Option<(usize, usize)>,
}

struct EditView {
    editor_state: TextEditorInteractionState,
    editor: TextEditor,
    find_field_state: TextFieldInteractionState,
    find_field: TextField,
    find_state: RefCell<FindState>,
    file_type: &'static str,
    encoding: &'static str,
}

#[derive(Clone, Copy)]
struct EditGeometry {
    editor: Rect,
    find_bar: Option<Rect>,
    find_field: Rect,
    find_result: Rect,
    footer: Rect,
}

impl EditView {
    fn new(
        editor_state: TextEditorInteractionState,
        file_type: &'static str,
        encoding: &'static str,
    ) -> Self {
        let find_field_state = TextFieldInteractionState::new();
        Self {
            editor: TextEditor::with_interaction(editor_state.clone())
                .placeholder("Start typing"),
            editor_state,
            find_field: TextField::with_interaction(find_field_state.clone())
                .placeholder("Find")
                .size(TextFieldSize::Small),
            find_field_state,
            find_state: RefCell::new(FindState::default()),
            file_type,
            encoding,
        }
    }

    fn geometry(&self, bounds: Rect, theme: &Theme) -> EditGeometry {
        let footer_height = theme.layout.status_bar_height.min(bounds.size.height);
        let footer = Rect::new(
            bounds.origin.x,
            bounds.origin.y + (bounds.size.height - footer_height).max(0.0),
            bounds.size.width,
            footer_height,
        );
        let find_visible = self.find_state.borrow().visible;
        let find_height = if find_visible {
            theme.layout.top_bar_height.min((bounds.size.height - footer_height).max(0.0))
        } else {
            0.0
        };
        let find_bar = find_visible.then(|| {
            Rect::new(bounds.origin.x, bounds.origin.y, bounds.size.width, find_height)
        });
        let horizontal = theme.spacing.large;
        let field_height = theme.layout.compact_control_height.min(find_height);
        let field_width = (bounds.size.width * 0.42)
            .clamp(180.0, 360.0)
            .min((bounds.size.width - horizontal * 2.0).max(0.0));
        let find_field = Rect::new(
            bounds.origin.x + horizontal,
            bounds.origin.y + (find_height - field_height) / 2.0,
            field_width,
            field_height,
        );
        let find_result = Rect::new(
            find_field.origin.x + find_field.size.width + theme.spacing.medium,
            find_field.origin.y,
            (bounds.origin.x + bounds.size.width
                - horizontal
                - find_field.origin.x
                - find_field.size.width
                - theme.spacing.medium)
                .max(0.0),
            field_height,
        );
        let editor = Rect::new(
            bounds.origin.x,
            bounds.origin.y + find_height,
            bounds.size.width,
            (bounds.size.height - footer_height - find_height).max(0.0),
        );
        EditGeometry {
            editor,
            find_bar,
            find_field,
            find_result,
            footer,
        }
    }

    fn update_find_result(&self, backwards: bool, from_start: bool) {
        let query = self.find_field_state.value();
        let result = self
            .editor_state
            .find_match(&query, backwards, from_start);
        let mut state = self.find_state.borrow_mut();
        state.last_query = query;
        state.result = result;
    }

    fn find_result_label(&self) -> String {
        let state = self.find_state.borrow();
        if state.last_query.is_empty() {
            String::new()
        } else if let Some((current, total)) = state.result {
            format!("{current} of {total}")
        } else {
            "No results".into()
        }
    }
}

impl View for EditView {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let geometry = self.geometry(bounds, context.theme);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.colors.surface))
            .paint(bounds, context);
        self.editor.paint(geometry.editor, context);

        if let Some(find_bar) = geometry.find_bar {
            Rectangle::new()
                .color(RectangleColor::Custom(context.theme.colors.surface_subtle))
                .paint(find_bar, context);
            self.find_field.paint(geometry.find_field, context);
            let result_bounds = vertically_centered_text(geometry.find_result, TextRole::Caption, context);
            Text::metadata(self.find_result_label()).paint(result_bounds, context);
            paint_top_or_bottom_divider(find_bar, false, context);
        }

        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.colors.surface_subtle))
            .paint(geometry.footer, context);
        paint_top_or_bottom_divider(geometry.footer, true, context);

        let horizontal = context.theme.spacing.large;
        let footer_text = vertically_centered_text(Rect::new(
            geometry.footer.origin.x + horizontal,
            geometry.footer.origin.y,
            (geometry.footer.size.width - horizontal * 2.0).max(0.0),
            geometry.footer.size.height,
        ), TextRole::Caption, context);
        Text::metadata(self.file_type).paint(footer_text, context);
        let (lines, characters) = self.editor_state.statistics();
        let line_label = if lines == 1 { "line" } else { "lines" };
        let character_label = if characters == 1 { "character" } else { "characters" };
        Text::metadata(format!(
            "{}    {} {}    {} {}",
            self.encoding, lines, line_label, characters, character_label
        ))
        .alignment(TextAlignment::End)
        .paint(footer_text, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let geometry = self.geometry(bounds, context.theme());
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('f' | 'F'),
                modifiers
            } if modifiers.shortcut()
        ) {
            self.find_state.borrow_mut().visible = true;
            self.find_field_state.set_focused(true);
            let _ = self.editor.handle_event(
                geometry.editor,
                &ViewEvent::KeyboardFocusRequested {
                    bounds: Some(geometry.find_field),
                },
                context,
            );
            context.request_redraw();
            return EventResult::Consumed;
        }

        if self.find_state.borrow().visible
            && matches!(event, ViewEvent::KeyPressed { key: Key::Escape, .. })
        {
            self.find_state.borrow_mut().visible = false;
            self.find_field_state.set_focused(false);
            context.request_keyboard_focus(geometry.editor);
            context.request_redraw();
            return EventResult::Consumed;
        }

        if self.find_state.borrow().visible
            && self.find_field_state.is_focused()
            && let ViewEvent::KeyPressed {
                key: Key::Enter,
                modifiers,
            } = event
        {
            self.update_find_result(modifiers.shift(), false);
            context.request_redraw();
            return EventResult::Consumed;
        }

        let revision = self.editor_state.revision();
        let mut result = EventResult::Ignored;
        if self.find_state.borrow().visible {
            result = result.merge(
                self.find_field
                    .handle_event(geometry.find_field, event, context),
            );
            let query = self.find_field_state.value();
            if query != self.find_state.borrow().last_query {
                self.update_find_result(false, true);
                context.request_redraw();
            }
        }
        result = result.merge(self.editor.handle_event(geometry.editor, event, context));
        if self.editor_state.revision() != revision {
            context.request_redraw_in(geometry.footer);
        }
        result
    }
}

fn paint_top_or_bottom_divider(bounds: Rect, top: bool, context: &mut PaintContext<'_>) {
    let thickness = context.theme.divider.thickness.max(1.0);
    let y = if top {
        bounds.origin.y
    } else {
        bounds.origin.y + (bounds.size.height - thickness).max(0.0)
    };
    context.display_list.push(DrawCommand::FillRect {
        rect: Rect::new(bounds.origin.x, y, bounds.size.width, thickness),
        color: context.theme.colors.border,
    });
}

fn vertically_centered_text(
    bounds: Rect,
    role: TextRole,
    context: &PaintContext<'_>,
) -> Rect {
    let line_height = context.typography.style(role).line_height
        * context.text_measurer.font_scale();
    Rect::new(
        bounds.origin.x,
        bounds.origin.y + ((bounds.size.height - line_height) / 2.0).max(0.0),
        bounds.size.width,
        line_height.min(bounds.size.height),
    )
}

fn main() -> Result<(), ViewKitError> {
    run::<EditApp>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_structured_text_types() {
        assert_eq!(file_type_for_path(Path::new("settings.json")), "JSON");
        assert_eq!(file_type_for_path(Path::new("version.toml")), "TOML");
        assert_eq!(file_type_for_path(Path::new("notes.txt")), "Plain Text");
        assert_eq!(file_type_for_path(Path::new("README")), "Plain Text");
    }
}
