mod preferences;

use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use appcore::prelude::*;
use appcore::viewkit::draw_command::DrawCommand;
use appcore::viewkit::event::{EventContext, EventResult, ViewEvent};
use appcore::viewkit::view::{Constraints, MeasureContext, PaintContext};
use preferences::EditorPreferences;

struct EditApp {
    editor: TextEditorInteractionState,
    document: DocumentController,
    preferences: EditorPreferenceModel,
    settings_visible: State<bool>,
}

impl App for EditApp {
    type Body = ApplicationMenuBar<EditView>;

    fn new() -> Self {
        let mut document = DocumentIdentity::from_arguments();
        let editor = TextEditorInteractionState::new();
        if let Some(contents) = document.contents.take() {
            editor.set_value(contents);
        }
        let preferences = EditorPreferenceModel::load();
        let document = make_document_controller(document, editor.clone());
        Self {
            editor,
            document,
            preferences,
            settings_visible: State::new(false),
        }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new(format!("{} — Edit", self.document.info().display_name))
            .size(860.0, 640.0)
            .resizable(true)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        let view = EditView::new(
            self.editor.clone(),
            self.document.clone(),
            self.preferences.clone(),
            self.settings_visible.clone(),
        );

        let open_document = self.document.clone();
        let save_document = self.document.clone();
        let save_as_document = self.document.clone();
        let quit_document = self.document.clone();

        let find_state = Rc::clone(&view.find_state);
        let find_field = view.find_field_state.clone();
        let show_find_state = Rc::clone(&view.find_state);
        let show_find_field = view.find_field_state.clone();
        let find_checked = Rc::clone(&view.find_state);

        let settings_visible = self.settings_visible.clone();
        let view_settings_visible = self.settings_visible.clone();

        ApplicationMenuBar::new(view)
            .menu(
                ApplicationMenu::new("File")
                    .item(
                        ApplicationMenuItem::new("Open…", move || open_document.open())
                            .shortcut(MenuShortcut::command('o', "Ctrl+O")),
                    )
                    .separator()
                    .item(
                        ApplicationMenuItem::new("Save", move || {
                            let _ = save_document.save();
                        })
                        .shortcut(MenuShortcut::command('s', "Ctrl+S")),
                    )
                    .item(
                        ApplicationMenuItem::new("Save As…", move || save_as_document.save_as())
                            .shortcut(MenuShortcut::command('s', "Ctrl+Shift+S").shift()),
                    )
                    .separator()
                    .item(
                        ApplicationMenuItem::new("Close", request_close_key_window)
                            .shortcut(MenuShortcut::command('w', "Ctrl+W")),
                    )
                    .separator()
                    .item(
                        ApplicationMenuItem::new("Quit Edit", move || {
                            if quit_document.request_close_with(request_quit) {
                                request_quit();
                            }
                        })
                        .shortcut(MenuShortcut::command('q', "Ctrl+Q")),
                    ),
            )
            .menu(
                ApplicationMenu::standard_edit()
                    .separator()
                    .item(
                        ApplicationMenuItem::new("Find…", move || {
                            find_state.borrow_mut().visible = true;
                            find_field.set_focused(true);
                        })
                        .shortcut(MenuShortcut::command('f', "Ctrl+F")),
                    )
                    .separator()
                    .item(
                        ApplicationMenuItem::new("Settings…", move || settings_visible.set(true))
                            .shortcut(MenuShortcut::command(',', "Ctrl+,")),
                    ),
            )
            .menu(
                ApplicationMenu::new("View")
                    .item(
                        ApplicationMenuItem::new("Show Find Bar", move || {
                            let visible = !show_find_state.borrow().visible;
                            show_find_state.borrow_mut().visible = visible;
                            show_find_field.set_focused(visible);
                        })
                        .checked_when(move || find_checked.borrow().visible),
                    )
                    .item(ApplicationMenuItem::new("Editor Settings…", move || {
                        view_settings_visible.set(true)
                    })),
            )
    }

    fn close_requested(&mut self) -> bool {
        self.document
            .request_close_with(|| appcore::viewkit::close_window(WindowId::PRIMARY))
    }
}

#[derive(Clone)]
struct DocumentIdentity {
    file_type: &'static str,
    encoding: &'static str,
    contents: Option<String>,
    path: Option<PathBuf>,
    writes_bom: bool,
    can_save: bool,
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
                file_type: "Plain Text",
                encoding: "UTF-8",
                contents: None,
                path: None,
                writes_bom: false,
                can_save: false,
            };
        };

        let file_type = file_type_for_path(&path);
        let (contents, encoding, writes_bom, can_save) = match std::fs::read(&path) {
            Ok(bytes) if bytes.starts_with(&[0xef, 0xbb, 0xbf]) => {
                match String::from_utf8(bytes[3..].to_vec()) {
                    Ok(contents) => (Some(contents), "UTF-8 with BOM", true, true),
                    Err(_) => (None, "Unsupported encoding", false, false),
                }
            }
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(contents) => (Some(contents), "UTF-8", false, true),
                Err(_) => (None, "Unsupported encoding", false, false),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                (Some(String::new()), "UTF-8", false, true)
            }
            Err(_) => (None, "Unavailable", false, false),
        };
        Self {
            file_type,
            encoding,
            contents,
            path: Some(path),
            writes_bom,
            can_save,
        }
    }
}

#[derive(Clone)]
struct EditorPreferenceModel {
    line_wrap: State<bool>,
    show_line_count: State<bool>,
    font_size: State<f32>,
    persisted: Rc<RefCell<EditorPreferences>>,
    save_error: Rc<RefCell<Option<String>>>,
}

impl EditorPreferenceModel {
    fn load() -> Self {
        let preferences = EditorPreferences::load();
        Self {
            line_wrap: State::new(preferences.line_wrap),
            show_line_count: State::new(preferences.show_line_count),
            font_size: State::new(preferences.font_size),
            persisted: Rc::new(RefCell::new(preferences)),
            save_error: Rc::new(RefCell::new(None)),
        }
    }

    fn snapshot(&self) -> EditorPreferences {
        EditorPreferences {
            line_wrap: self.line_wrap.get(),
            show_line_count: self.show_line_count.get(),
            font_size: self.font_size.get(),
        }
    }

    fn persist_if_changed(&self) {
        let preferences = self.snapshot();
        if *self.persisted.borrow() == preferences {
            return;
        }
        match preferences.save() {
            Ok(()) => {
                *self.persisted.borrow_mut() = preferences;
                self.save_error.borrow_mut().take();
            }
            Err(error) => {
                *self.save_error.borrow_mut() = Some(format!("Unable to save settings: {error}"));
            }
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

fn save_document(path: &Path, contents: &str, writes_bom: bool) -> io::Result<()> {
    // A Save panel grants this process the selected file, not the surrounding
    // directory. Write that file directly rather than creating a sibling
    // temporary file outside the granted scope.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    if writes_bom {
        file.write_all(&[0xef, 0xbb, 0xbf])?;
    }
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

fn make_document_controller(
    document: DocumentIdentity,
    editor: TextEditorInteractionState,
) -> DocumentController {
    let metadata = DocumentMetadata::new(document.file_type, document.encoding)
        .writable(document.can_save || document.path.is_none());
    let info = match document.path {
        Some(path) => DocumentInfo::at(path, metadata),
        None => DocumentInfo::untitled(metadata),
    };
    let writes_bom = Rc::new(Cell::new(document.writes_bom));

    let revision_editor = editor.clone();
    let open_editor = editor.clone();
    let open_writes_bom = Rc::clone(&writes_bom);
    let save_editor = editor;
    let save_writes_bom = Rc::clone(&writes_bom);

    DocumentController::new(
        info,
        None,
        move || revision_editor.revision(),
        move |path| {
            let bytes = fs::read(path).map_err(|error| format!("Unable to open: {error}"))?;
            let (contents, encoding, has_bom) = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
                (
                    String::from_utf8(bytes[3..].to_vec())
                        .map_err(|_| String::from("This file is not valid UTF-8 text."))?,
                    "UTF-8 with BOM",
                    true,
                )
            } else {
                (
                    String::from_utf8(bytes)
                        .map_err(|_| String::from("This file is not valid UTF-8 text."))?,
                    "UTF-8",
                    false,
                )
            };
            open_editor.set_value(contents);
            open_writes_bom.set(has_bom);
            Ok(DocumentMetadata::new(file_type_for_path(path), encoding))
        },
        move |path| {
            let has_bom = save_writes_bom.get();
            save_document(path, &save_editor.value(), has_bom)
                .map_err(|error| error.to_string())?;
            Ok(DocumentMetadata::new(
                file_type_for_path(path),
                if has_bom { "UTF-8 with BOM" } else { "UTF-8" },
            ))
        },
    )
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
    find_state: Rc<RefCell<FindState>>,
    document: DocumentController,
    preferences: EditorPreferenceModel,
    settings_visible: State<bool>,
    settings: EditSettingsView,
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
        document: DocumentController,
        preferences: EditorPreferenceModel,
        settings_visible: State<bool>,
    ) -> Self {
        let find_field_state = TextFieldInteractionState::new();
        let settings = EditSettingsView::new(preferences.clone(), settings_visible.clone());
        Self {
            editor: TextEditor::with_interaction(editor_state.clone())
                .placeholder("Start typing")
                .monospaced(true)
                .line_wrap(preferences.line_wrap.get())
                .font_size(preferences.font_size.get()),
            editor_state,
            find_field: TextField::with_interaction(find_field_state.clone())
                .placeholder("Find")
                .size(TextFieldSize::Small),
            find_field_state,
            find_state: Rc::new(RefCell::new(FindState::default())),
            document,
            preferences,
            settings_visible,
            settings,
        }
    }

    fn save(&self) -> bool {
        self.document.save()
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
            theme
                .layout
                .top_bar_height
                .min((bounds.size.height - footer_height).max(0.0))
        } else {
            0.0
        };
        let find_bar = find_visible.then(|| {
            Rect::new(
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                find_height,
            )
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
        let result = self.editor_state.find_match(&query, backwards, from_start);
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
        if self.settings_visible.get() {
            self.settings.paint(bounds, context);
            if self.document.is_presenting() {
                self.document.paint(bounds, context);
            }
            return;
        }
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
            let result_bounds =
                vertically_centered_text(geometry.find_result, TextRole::Caption, context);
            Text::metadata(self.find_result_label()).paint(result_bounds, context);
            paint_top_or_bottom_divider(find_bar, false, context);
        }

        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.colors.surface_subtle))
            .paint(geometry.footer, context);
        paint_top_or_bottom_divider(geometry.footer, true, context);

        let horizontal = context.theme.spacing.large;
        let footer_text = vertically_centered_text(
            Rect::new(
                geometry.footer.origin.x + horizontal,
                geometry.footer.origin.y,
                (geometry.footer.size.width - horizontal * 2.0).max(0.0),
                geometry.footer.size.height,
            ),
            TextRole::Caption,
            context,
        );
        let document = self.document.info();
        let mut file_status = document.metadata.file_type.clone();
        if let Some(status) = self.document.status() {
            file_status.push_str("    ");
            file_status.push_str(&status);
        } else if self.document.is_edited() {
            file_status.push_str("    Edited");
        }
        Text::metadata(file_status).paint(footer_text, context);
        let (lines, characters) = self.editor_state.statistics();
        let character_label = if characters == 1 {
            "character"
        } else {
            "characters"
        };
        let statistics = if self.preferences.show_line_count.get() {
            let line_label = if lines == 1 { "line" } else { "lines" };
            format!(
                "{}    {} {}    {} {}",
                document.metadata.encoding, lines, line_label, characters, character_label
            )
        } else {
            format!(
                "{}    {} {}",
                document.metadata.encoding, characters, character_label
            )
        };
        Text::metadata(statistics)
            .alignment(TextAlignment::End)
            .paint(footer_text, context);
        if self.document.is_presenting() {
            self.document.paint(bounds, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if self.document.is_presenting() {
            return self.document.handle_event(bounds, event, context);
        }

        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('o' | 'O'),
                modifiers
            } if modifiers.shortcut()
        ) {
            self.document.open();
            context.request_redraw();
            return EventResult::Consumed;
        }
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('w' | 'W'),
                modifiers
            } if modifiers.shortcut()
        ) {
            request_close_key_window();
            context.request_redraw();
            return EventResult::Consumed;
        }
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('q' | 'Q'),
                modifiers
            } if modifiers.shortcut()
        ) {
            if self.document.request_close_with(request_quit) {
                request_quit();
            }
            context.request_redraw();
            return EventResult::Consumed;
        }

        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character(','),
                modifiers
            } if modifiers.shortcut()
        ) {
            self.settings_visible.set(!self.settings_visible.get());
            context.request_redraw();
            return EventResult::Consumed;
        }

        if self.settings_visible.get() {
            if matches!(
                event,
                ViewEvent::KeyPressed {
                    key: Key::Escape,
                    ..
                }
            ) {
                self.settings_visible.set(false);
                context.request_redraw();
                return EventResult::Consumed;
            }
            let result = self.settings.handle_event(bounds, event, context);
            self.preferences.persist_if_changed();
            return result;
        }

        let geometry = self.geometry(bounds, context.theme());
        if matches!(event, ViewEvent::FocusChanged { focused: true })
            && !self.find_state.borrow().visible
        {
            context.request_keyboard_focus(geometry.editor);
        }

        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('s' | 'S'),
                modifiers
            } if modifiers.shortcut() && modifiers.shift()
        ) {
            self.document.save_as();
            context.request_redraw();
            return EventResult::Consumed;
        }
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('s' | 'S'),
                modifiers
            } if modifiers.shortcut()
        ) {
            let _ = self.save();
            context.request_redraw_in(geometry.footer);
            return EventResult::Consumed;
        }
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
            && matches!(
                event,
                ViewEvent::KeyPressed {
                    key: Key::Escape,
                    ..
                }
            )
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
            result = result.merge(self.find_field.handle_event(
                geometry.find_field,
                event,
                context,
            ));
            let query = self.find_field_state.value();
            if query != self.find_state.borrow().last_query {
                self.update_find_result(false, true);
                context.request_redraw();
            }
        }
        result = result.merge(self.editor.handle_event(geometry.editor, event, context));
        if self.editor_state.revision() != revision {
            self.document.clear_status();
            context.request_redraw_in(geometry.footer);
        }
        result
    }
}

struct EditSettingsView {
    page: SettingsPage<FormSections>,
    done: Button,
    preferences: EditorPreferenceModel,
}

impl EditSettingsView {
    fn new(preferences: EditorPreferenceModel, visible: State<bool>) -> Self {
        let font_size = preferences.font_size.get();
        let font_control = HStack::new()
            .gap(StackGap::Medium)
            .child(
                Slider::new(preferences.font_size.binding())
                    .range(10.0..=32.0)
                    .step(1.0)
                    .frame(220.0, Theme::current().layout.control_height),
            )
            .child(
                Text::metadata(format!("{font_size:.0} pt"))
                    .frame(48.0, Theme::current().layout.control_height),
            );
        let page = SettingsPage::form("Editor")
            .subtitle("Choose how documents are displayed while editing.")
            .section(
                SettingsSection::new("Text Editing")
                    .row(
                        SettingsRow::new(
                            "Line Wrapping",
                            Switch::new(preferences.line_wrap.binding()),
                        )
                        .description("Wrap long lines to the width of the window"),
                    )
                    .row(
                        SettingsRow::new("Text Size", font_control)
                            .description("Size of document text"),
                    ),
            )
            .section(
                SettingsSection::new("Status Bar").row(
                    SettingsRow::new(
                        "Show Line Count",
                        Switch::new(preferences.show_line_count.binding()),
                    )
                    .description("Show the number of lines in the document footer"),
                ),
            );
        let done = Button::new("Done")
            .size(ButtonSize::Small)
            .style(ButtonStyle::Ghost)
            .on_click(move || visible.set(false));
        Self {
            page,
            done,
            preferences,
        }
    }

    fn geometry(bounds: Rect, theme: &Theme) -> (Rect, Rect, Rect) {
        let bar_height = theme.layout.top_bar_height.min(bounds.size.height);
        let bar = Rect::new(
            bounds.origin.x,
            bounds.origin.y,
            bounds.size.width,
            bar_height,
        );
        let done = Rect::new(
            bounds.origin.x + bounds.size.width - theme.spacing.large - 72.0,
            bounds.origin.y + (bar_height - theme.layout.compact_control_height) / 2.0,
            72.0,
            theme.layout.compact_control_height,
        );
        let content = Rect::new(
            bounds.origin.x + theme.spacing.extra_large,
            bounds.origin.y + bar_height + theme.spacing.extra_large,
            (bounds.size.width - theme.spacing.extra_large * 2.0).max(0.0),
            (bounds.size.height - bar_height - theme.spacing.extra_large * 2.0).max(0.0),
        );
        (bar, done, content)
    }
}

impl View for EditSettingsView {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let (bar, done, content) = Self::geometry(bounds, context.theme);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.colors.surface))
            .paint(bounds, context);
        let title = vertically_centered_text(bar, TextRole::Label, context);
        Text::label("Settings")
            .weight(600)
            .alignment(TextAlignment::Center)
            .paint(title, context);
        self.done.paint(done, context);
        paint_top_or_bottom_divider(bar, false, context);
        self.page.paint(content, context);

        if let Some(error) = self.preferences.save_error.borrow().as_ref() {
            let status = Rect::new(
                content.origin.x,
                bounds.origin.y + bounds.size.height - context.theme.layout.status_bar_height,
                content.size.width,
                context.theme.layout.status_bar_height,
            );
            Text::metadata(error.clone()).paint(status, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let (_, done, content) = Self::geometry(bounds, context.theme());
        self.done
            .handle_event(done, event, context)
            .merge(self.page.handle_event(content, event, context))
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

fn vertically_centered_text(bounds: Rect, role: TextRole, context: &PaintContext<'_>) -> Rect {
    let line_height =
        context.typography.style(role).line_height * context.text_measurer.font_scale();
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
    use appcore::viewkit::platform::KeyModifiers;
    use appcore::viewkit::typography::TextMeasurer;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recognizes_structured_text_types() {
        assert_eq!(file_type_for_path(Path::new("settings.json")), "JSON");
        assert_eq!(file_type_for_path(Path::new("version.toml")), "TOML");
        assert_eq!(file_type_for_path(Path::new("notes.txt")), "Plain Text");
        assert_eq!(file_type_for_path(Path::new("README")), "Plain Text");
    }

    #[test]
    fn save_preserves_utf8_bom_and_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("mochios-edit-save-{}-{unique}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("document.txt");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        save_document(&path, "new\ntext", true).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbfnew\ntext");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_shortcut_writes_the_current_editor_contents() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mochios-edit-shortcut-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("document.txt");
        fs::write(&path, b"old").unwrap();

        let editor = TextEditorInteractionState::new();
        editor.set_value("saved from shortcut");
        editor.focus();
        let preferences = EditorPreferences::default();
        let preference_model = EditorPreferenceModel {
            line_wrap: State::new(preferences.line_wrap),
            show_line_count: State::new(preferences.show_line_count),
            font_size: State::new(preferences.font_size),
            persisted: Rc::new(RefCell::new(preferences)),
            save_error: Rc::new(RefCell::new(None)),
        };
        let document = make_document_controller(
            DocumentIdentity {
                file_type: "Plain Text",
                encoding: "UTF-8",
                contents: None,
                path: Some(path.clone()),
                writes_bom: false,
                can_save: true,
            },
            editor.clone(),
        );
        let view = EditView::new(editor, document, preference_model, State::new(false));
        let theme = Theme::LIGHT;
        let mut text_measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text_measurer);

        let result = view.handle_event(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            &ViewEvent::KeyPressed {
                key: Key::Character('s'),
                modifiers: KeyModifiers::from_bits(KeyModifiers::CONTROL),
            },
            &mut context,
        );

        assert_eq!(result, EventResult::Consumed);
        assert_eq!(fs::read_to_string(path).unwrap(), "saved from shortcut");
        fs::remove_dir_all(root).unwrap();
    }
}
