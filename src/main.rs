mod preferences;
mod preview;

use std::cell::{Cell, RefCell};
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use appcore::prelude::*;
use appcore::viewkit::event::{EventContext, EventResult, ViewEvent};
use appcore::viewkit::view::{Constraints, MeasureContext, PaintContext};
use preferences::EditorPreferences;

struct EditApp {
    editor: TextEditorInteractionState,
    document: DocumentController,
    preferences: EditorPreferenceModel,
    settings_visible: State<bool>,
    launch_name: Option<String>,
}

impl App for EditApp {
    type Body = ApplicationMenuBar<EditView>;

    fn new() -> Self {
        let mut document = DocumentIdentity::from_arguments();
        let startup_path = document.path.take();
        let launch_name = startup_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let editor = TextEditorInteractionState::new();
        if let Some(contents) = document.contents.take() {
            editor.set_value(contents);
        }
        let preferences = EditorPreferenceModel::load();
        let document = make_document_controller(document, editor.clone());
        if let Some(path) = startup_path {
            document.open_path(path);
        }
        Self {
            editor,
            document,
            preferences,
            settings_visible: State::new(false),
            launch_name,
        }
    }

    fn window(&self) -> WindowOptions {
        let name = self
            .launch_name
            .clone()
            .unwrap_or_else(|| self.document.info().display_name);
        WindowOptions::new(format!("{name} — Edit"))
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
                            let _ = save_document.request_save();
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
                        ApplicationMenuItem::new("Quit Edit", request_quit)
                            .shortcut(MenuShortcut::command('q', "Ctrl+Q")),
                    ),
            )
            .menu(
                ApplicationMenu::standard_edit()
                    .separator()
                    .item(
                        ApplicationMenuItem::new("Find…", move || {
                            let mut state = find_state.borrow_mut();
                            state.visible = true;
                            state.request_focus(EditFocus::Find);
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
                            let mut state = show_find_state.borrow_mut();
                            state.visible = visible;
                            state.request_focus(if visible {
                                EditFocus::Find
                            } else {
                                EditFocus::Editor
                            });
                            show_find_field.set_focused(visible);
                        })
                        .checked_when(move || find_checked.borrow().visible),
                    )
                    .item(ApplicationMenuItem::new("Editor Settings…", move || {
                        view_settings_visible.set(true)
                    })),
            )
    }

    fn handle_platform_message_with_handles(
        &mut self,
        message: &[u8],
        handles: &mut [appcore::viewkit::platform::PlatformFileHandle],
    ) -> bool {
        match appcore::document::decode_open_message(message, handles) {
            Ok(Some(document)) => {
                if let Err(error) = self
                    .document
                    .open_file(DocumentFile::from_received(document))
                {
                    Alert::new().present_error("The document could not be opened.", error);
                }
                true
            }
            Ok(None) => false,
            Err(error) => {
                Alert::new().present_error(
                    "The document could not be opened.",
                    format!("Invalid document delivery: {error:?}"),
                );
                true
            }
        }
    }

    fn close_requested(&mut self) -> bool {
        self.document.request_close_with(request_quit)
    }
    fn close_requested_for_window(&mut self, window: WindowId) -> bool {
        self.document
            .request_close_with(move || appcore::viewkit::close_window(window))
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
        Self {
            file_type,
            encoding: "UTF-8",
            contents: None,
            path: Some(path),
            writes_bom: false,
            can_save: false,
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

struct TextContent {
    text: String,
    writes_bom: bool,
}
struct TextFormat;
impl DocumentFormat for TextFormat {
    type Content = TextContent;
    fn read(&self, file: &DocumentFile) -> Result<(TextContent, DocumentMetadata), String> {
        // Application limit, not an OS-wide document size specification.
        let mut bytes = file
            .read_limited(64 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let writes_bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        if writes_bom {
            bytes.drain(..3);
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| "This file is not valid UTF-8 text.".to_owned())?;
        let metadata = DocumentMetadata::new(
            file_type_for_path(file.path()),
            if writes_bom {
                "UTF-8 with BOM"
            } else {
                "UTF-8"
            },
        );
        Ok((TextContent { text, writes_bom }, metadata))
    }
    fn write(&self, file: &DocumentFile, content: TextContent) -> Result<DocumentMetadata, String> {
        let mut bytes = Vec::new();
        if content.writes_bom {
            bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
        }
        bytes.extend_from_slice(content.text.as_bytes());
        file.replace_contents(&bytes)
            .map_err(|error| error.to_string())?;
        Ok(DocumentMetadata::new(
            file_type_for_path(file.path()),
            if content.writes_bom {
                "UTF-8 with BOM"
            } else {
                "UTF-8"
            },
        ))
    }
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
    let fallback_editor = editor.clone();
    let snapshot_editor = editor.clone();
    let snapshot_bom = writes_bom.clone();
    DocumentController::new_background(
        info,
        None,
        move || revision_editor.revision(),
        DocumentIo::new(
            TextFormat,
            move || {
                Ok(TextContent {
                    text: snapshot_editor.value(),
                    writes_bom: snapshot_bom.get(),
                })
            },
            move |content| {
                editor.set_value(content.text);
                writes_bom.set(content.writes_bom);
            },
        ),
    )
    .unwrap_or_else(|error| {
        // Fail closed: never fall back to blocking I/O on the UI thread.
        DocumentController::new(
            DocumentInfo::untitled(DocumentMetadata::new("Plain Text", "UTF-8")),
            None,
            move || fallback_editor.revision(),
            {
                let error = error.to_string();
                move |_| Err(format!("Document workers are unavailable: {error}"))
            },
            move |_| Err(format!("Document workers are unavailable: {error}")),
        )
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditFocus {
    Editor,
    Find,
}

#[derive(Default)]
struct FindState {
    visible: bool,
    last_query: String,
    result: Option<(usize, usize)>,
    focus_after_layout: Option<EditFocus>,
    focus_bounds: Option<Rect>,
}
impl FindState {
    fn request_focus(&mut self, target: EditFocus) {
        self.focus_after_layout = Some(target);
        self.focus_bounds = None;
    }
}

struct EditView {
    editor_state: TextEditorInteractionState,
    editor: Rc<TextEditor>,
    find_field_state: TextFieldInteractionState,
    find_field: Rc<TextField>,
    find_state: Rc<RefCell<FindState>>,
    document: DocumentController,
    preferences: EditorPreferenceModel,
    settings_visible: State<bool>,
    settings: EditSettingsView,
    editor_bounds: Rc<Cell<Option<Rect>>>,
    footer_bounds: Rc<Cell<Option<Rect>>>,
}

// Observe the bounds assigned by ViewKit; never compute a second layout for
// focus requests or partial status redraws.
struct LayoutBounds<V> {
    content: V,
    bounds: Option<Rc<Cell<Option<Rect>>>>,
    focus: Option<EditFocus>,
    find_state: Rc<RefCell<FindState>>,
}
impl<V: View> View for LayoutBounds<V> {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content.measure(constraints, context)
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        if let Some(record) = &self.bounds {
            record.set(Some(bounds));
        }
        if let Some(target) = self.focus {
            let mut state = self.find_state.borrow_mut();
            if state.focus_after_layout == Some(target) {
                state.focus_bounds = Some(bounds);
            }
        }
        self.content.paint(bounds, context);
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if let Some(record) = &self.bounds {
            record.set(Some(bounds));
        }
        self.content.handle_event(bounds, event, context)
    }
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
            editor: Rc::new(
                TextEditor::with_interaction(editor_state.clone())
                    .placeholder("Start typing")
                    .monospaced(true)
                    .line_wrap(preferences.line_wrap.get())
                    .font_size(preferences.font_size.get()),
            ),
            editor_state,
            find_field: Rc::new(
                TextField::with_interaction(find_field_state.clone())
                    .placeholder("Find")
                    .size(TextFieldSize::Small),
            ),
            find_field_state,
            find_state: Rc::new(RefCell::new(FindState::default())),
            document,
            preferences,
            settings_visible,
            settings,
            editor_bounds: Rc::new(Cell::new(None)),
            footer_bounds: Rc::new(Cell::new(None)),
        }
    }

    fn save(&self) -> SaveRequest {
        self.document.request_save()
    }

    fn find_bar(&self, theme: &Theme) -> Toolbar<HStack> {
        Toolbar::new(
            HStack::new()
                .alignment(StackAlignment::Center)
                .gap(StackGap::Medium)
                .child(
                    LayoutBounds {
                        content: self.find_field.clone(),
                        bounds: None,
                        focus: Some(EditFocus::Find),
                        find_state: self.find_state.clone(),
                    }
                    .width(theme.layout.form_control_width),
                )
                .child(
                    Text::metadata(self.find_result_label())
                        .single_line(true)
                        .layout()
                        .flex_grow(1.0)
                        .flex_shrink(1.0),
                ),
        )
    }

    fn content(&self, theme: &Theme) -> Surface<VStack> {
        let mut layout = VStack::new()
            .alignment(StackAlignment::Stretch)
            .gap(StackGap::None);
        if self.find_state.borrow().visible {
            layout = layout.child(self.find_bar(theme).layout().flex_shrink(0.0));
        }
        layout = layout.child(
            Surface::pane()
                .content(LayoutBounds {
                    content: self.editor.clone(),
                    bounds: Some(self.editor_bounds.clone()),
                    focus: Some(EditFocus::Editor),
                    find_state: self.find_state.clone(),
                })
                .layout()
                .flex_grow(1.0),
        );
        let footer = Surface::sidebar()
            .content(Padding::symmetric(theme.spacing.large, 0.0).content(self.status_bar()));
        layout = layout.child(
            LayoutBounds {
                content: footer,
                bounds: Some(self.footer_bounds.clone()),
                focus: None,
                find_state: self.find_state.clone(),
            }
            .height(theme.layout.status_bar_height)
            .flex_shrink(0.0),
        );
        Surface::app().content(layout)
    }

    fn redraw_status(&self, context: &mut EventContext<'_>) {
        if let Some(bounds) = self.footer_bounds.get() {
            context.request_redraw_in(bounds);
        } else {
            context.request_redraw();
        }
    }

    fn status_bar(&self) -> HStack {
        let document = self.document.info();
        let mut file_status = document.metadata.file_type.clone();
        if let Some(status) = self.document.status() {
            file_status.push_str("    ");
            file_status.push_str(&status);
        } else if self.document.is_edited() {
            file_status.push_str("    Edited");
        }
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
        HStack::new()
            .alignment(StackAlignment::Center)
            .gap(StackGap::Large)
            .child(
                Text::metadata(file_status)
                    .single_line(true)
                    .layout()
                    .flex_grow(1.0)
                    .flex_shrink(1.0),
            )
            .child(
                Text::metadata(statistics)
                    .single_line(true)
                    .alignment(TextAlignment::End)
                    .layout()
                    .flex_shrink(1.0),
            )
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
        self.content(context.theme).paint(bounds, context);
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
            request_quit();
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

        if matches!(
            event,
            ViewEvent::PointerFocusRequested { .. } | ViewEvent::FocusChanged { focused: false }
        ) {
            let mut state = self.find_state.borrow_mut();
            state.focus_after_layout = None;
            state.focus_bounds = None;
        } else if !matches!(event, ViewEvent::KeyboardFocusRequested { .. }) {
            // Opening/closing Find changes the editor bounds. Wait until paint
            // has registered the new target in ViewKit's accessibility order;
            // requesting the old rectangle would lose focus on the next frame.
            let target = {
                let mut state = self.find_state.borrow_mut();
                let bounds = state.focus_bounds.take();
                if bounds.is_some() {
                    state.focus_after_layout = None;
                }
                bounds
            };
            if let Some(target) = target {
                self.content(context.theme()).handle_event(
                    bounds,
                    &ViewEvent::KeyboardFocusRequested {
                        bounds: Some(target),
                    },
                    context,
                );
                context.request_keyboard_focus(target);
            }
        }

        if matches!(event, ViewEvent::FocusChanged { focused: true })
            && !self.find_state.borrow().visible
        {
            context.request_keyboard_focus(self.editor_bounds.get().unwrap_or(bounds));
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
            self.redraw_status(context);
            return EventResult::Consumed;
        }
        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('f' | 'F'),
                modifiers
            } if modifiers.shortcut()
        ) {
            {
                let mut state = self.find_state.borrow_mut();
                state.visible = true;
                state.request_focus(EditFocus::Find);
            }
            self.find_field_state.set_focused(true);
            let _ = self.editor.handle_event(
                self.editor_bounds.get().unwrap_or(bounds),
                &ViewEvent::KeyboardFocusRequested { bounds: None },
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
            {
                let mut state = self.find_state.borrow_mut();
                state.visible = false;
                state.request_focus(EditFocus::Editor);
            }
            self.find_field_state.set_focused(false);
            self.editor_state.focus();
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
        let result = self
            .content(context.theme())
            .handle_event(bounds, event, context);
        if self.find_state.borrow().visible {
            let query = self.find_field_state.value();
            if query != self.find_state.borrow().last_query {
                self.update_find_result(false, true);
                context.request_redraw();
            }
        }
        if self.editor_state.revision() != revision {
            self.document.clear_status();
            self.redraw_status(context);
        }
        result
    }
}

struct EditSettingsView {
    page: Rc<SettingsPage<FormSections>>,
    done: ButtonInteractionState,
    toolbar: ActionToolbarState<()>,
    visible: State<bool>,
    scroll: ScrollState,
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
                        .compact()
                        .description("Wrap long lines to the width of the window"),
                    )
                    .row(
                        SettingsRow::new("Text Size", font_control)
                            .compact()
                            .description("Size of document text"),
                    ),
            )
            .section(
                SettingsSection::new("Status Bar").row(
                    SettingsRow::new(
                        "Show Line Count",
                        Switch::new(preferences.show_line_count.binding()),
                    )
                    .compact()
                    .description("Show the number of lines in the document footer"),
                ),
            );
        Self {
            page: Rc::new(page),
            done: ButtonInteractionState::new(),
            toolbar: ActionToolbarState::new(),
            visible,
            scroll: ScrollState::new(),
            preferences,
        }
    }

    fn content(&self) -> VStack {
        let visible = self.visible.clone();
        let actions = ActionToolbar::new(self.toolbar.clone())
            .title("Settings")
            .trailing(
                (),
                Button::with_interaction_and_label(self.done.clone(), "Done")
                    .on_click(move || visible.set(false)),
            );
        let mut content = VStack::new()
            .alignment(StackAlignment::Stretch)
            .gap(StackGap::None)
            .child(Toolbar::new(actions))
            .child(
                Surface::pane()
                    .content(
                        Scroll::new(self.scroll.clone())
                            .axis(ScrollAxis::Vertical)
                            .content(
                                Padding::all(Theme::current().spacing.extra_large)
                                    .content(self.page.clone()),
                            ),
                    )
                    .layout()
                    .flex_grow(1.0)
                    .flex_shrink(1.0),
            );
        if let Some(error) = self.preferences.save_error.borrow().as_ref() {
            content = content.child(
                Padding::symmetric(
                    Theme::current().spacing.large,
                    Theme::current().spacing.small,
                )
                .content(Text::metadata(error.clone()).color(Theme::current().colors.destructive))
                .layout()
                .flex_shrink(0.0),
            );
        }
        content
    }
}

impl View for EditSettingsView {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        Surface::app()
            .content(self.content())
            .paint(bounds, context);
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        Surface::app()
            .content(self.content())
            .handle_event(bounds, event, context)
    }
}

fn main() -> Result<(), ViewKitError> {
    match appcore::document::preview::serve(preview::generate) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(error) => {
            eprintln!("Preview failed: {error}");
            std::process::exit(1);
        }
    }
    run::<EditApp>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use appcore::viewkit::platform::KeyModifiers;
    use appcore::viewkit::typography::TextMeasurer;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn declared_document_icons_decode_and_rasterize() {
        for data in [
            include_bytes!("../resources/icons/text.svg").as_slice(),
            include_bytes!("../resources/icons/c.svg").as_slice(),
            include_bytes!("../resources/icons/cpp.svg").as_slice(),
            include_bytes!("../resources/icons/rust.svg").as_slice(),
            include_bytes!("../resources/icons/markdown.svg").as_slice(),
            include_bytes!("../resources/icons/json.svg").as_slice(),
        ] {
            let svg = appcore::viewkit::svg::SvgData::decode(data).unwrap();
            let image = appcore::viewkit::image::ImageData::from_svg(&svg, 72, 72).unwrap();
            assert_eq!(image.width(), 72);
        }
    }

    #[test]
    fn recognizes_structured_text_types() {
        assert_eq!(file_type_for_path(Path::new("settings.json")), "JSON");
        assert_eq!(file_type_for_path(Path::new("version.toml")), "TOML");
        assert_eq!(file_type_for_path(Path::new("notes.txt")), "Plain Text");
        assert_eq!(file_type_for_path(Path::new("README")), "Plain Text");
    }

    #[test]
    fn background_text_format_preserves_bom_and_rejects_invalid_utf8() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("edit-format-{}-{nonce}.txt", std::process::id()));
        fs::write(&path, b"\xef\xbb\xbfhello").unwrap();
        let file = DocumentFile::open(&path).unwrap();
        let (content, metadata) = TextFormat.read(&file).unwrap();
        assert_eq!(content.text, "hello");
        assert!(content.writes_bom);
        assert_eq!(metadata.encoding, "UTF-8 with BOM");
        file.replace_contents(&[0xff, 0xfe]).unwrap();
        assert!(TextFormat.read(&file).is_err());
        fs::remove_file(path).unwrap();
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

        TextFormat
            .write(
                &DocumentFile::create(&path).unwrap(),
                TextContent {
                    text: "new\ntext".into(),
                    writes_bom: true,
                },
            )
            .unwrap();

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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while fs::read_to_string(&path).unwrap_or_default() != "saved from shortcut" {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        fs::remove_dir_all(root).unwrap();
    }
    fn fixture_view() -> EditView {
        let editor = TextEditorInteractionState::new();
        editor.set_value("alpha βeta alpha");
        editor.focus();
        let preferences = EditorPreferences::default();
        let model = EditorPreferenceModel {
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
                path: None,
                writes_bom: false,
                can_save: false,
            },
            editor.clone(),
        );
        EditView::new(editor, document, model, State::new(false))
    }

    fn snapshot(
        view: &EditView,
        bounds: Rect,
        theme: &Theme,
    ) -> Vec<appcore::viewkit::accessibility::AccessibilityNode> {
        let mut list = appcore::viewkit::draw_command::DisplayList::new();
        let mut nodes = Vec::new();
        let mut text = TextMeasurer::new();
        view.paint(
            bounds,
            &mut PaintContext::new(&mut list, theme, &theme.typography, &mut text)
                .with_accessibility_nodes(&mut nodes),
        );
        nodes
    }

    #[test]
    fn native_find_bar_preserves_search_navigation_and_document_contents() {
        let view = fixture_view();
        let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
        let theme = Theme::LIGHT;
        let mut text = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text);
        view.handle_event(
            bounds,
            &ViewEvent::KeyPressed {
                key: Key::Character('f'),
                modifiers: KeyModifiers::from_bits(KeyModifiers::CONTROL),
            },
            &mut context,
        );
        assert!(view.find_state.borrow().visible);
        assert!(view.find_field_state.is_focused());
        view.handle_event(
            bounds,
            &ViewEvent::TextInput {
                text: "alpha".into(),
            },
            &mut context,
        );
        assert_eq!(view.find_state.borrow().result, Some((1, 2)));
        view.handle_event(
            bounds,
            &ViewEvent::KeyPressed {
                key: Key::Enter,
                modifiers: KeyModifiers::default(),
            },
            &mut context,
        );
        assert_eq!(view.find_state.borrow().result, Some((2, 2)));
        view.handle_event(
            bounds,
            &ViewEvent::KeyPressed {
                key: Key::Escape,
                modifiers: KeyModifiers::default(),
            },
            &mut context,
        );
        assert!(!view.find_state.borrow().visible);
        assert_eq!(view.editor_state.value(), "alpha βeta alpha");
    }

    #[test]
    fn native_find_geometry_matches_focus_and_footer_labels_do_not_overlap() {
        let view = fixture_view();
        view.find_state.borrow_mut().visible = true;
        for theme in [Theme::LIGHT, Theme::DARK] {
            for width in [240.0, 360.0, 800.0] {
                let bounds = Rect::new(0.0, 0.0, width, 600.0);
                let nodes = snapshot(&view, bounds, &theme);
                let field = nodes
                    .iter()
                    .find(|n| {
                        n.label.as_deref() == Some("Find") && n.role == AccessibilityRole::TextField
                    })
                    .unwrap();
                let toolbar = nodes
                    .iter()
                    .find(|n| n.role == AccessibilityRole::Toolbar)
                    .unwrap()
                    .bounds;
                assert!(field.bounds.origin.x >= toolbar.origin.x);
                assert!(
                    field.bounds.origin.x + field.bounds.size.width
                        <= toolbar.origin.x + toolbar.size.width
                );
                assert!(
                    (field.bounds.origin.y + field.bounds.size.height * 0.5
                        - toolbar.origin.y
                        - toolbar.size.height * 0.5)
                        .abs()
                        < 0.1
                );
                let footer = view.footer_bounds.get().unwrap();
                let labels: Vec<_> = nodes
                    .iter()
                    .filter(|n| {
                        n.role == AccessibilityRole::StaticText
                            && n.bounds.origin.y >= footer.origin.y
                    })
                    .collect();
                assert_eq!(labels.len(), 2);
                assert!(labels[0].bounds.intersection(labels[1].bounds).is_none());
                for label in labels {
                    assert!(label.bounds.origin.x >= 0.0);
                    assert!(label.bounds.origin.x + label.bounds.size.width <= width + 0.01);
                }
            }
        }
    }

    #[test]
    fn standard_stack_keeps_editor_and_chrome_separate_during_resize() {
        let view = fixture_view();
        for theme in [Theme::LIGHT, Theme::DARK] {
            for visible in [false, true] {
                view.find_state.borrow_mut().visible = visible;
                for width in [240., 360., 800.] {
                    for height in [140., 320., 600.] {
                        let bounds = Rect::new(13., 17., width, height);
                        let nodes = snapshot(&view, bounds, &theme);
                        let editor = view.editor_bounds.get().unwrap();
                        let footer = view.footer_bounds.get().unwrap();
                        assert!(editor.size.width > 0. && editor.size.height > 0.);
                        assert!(editor.origin.y >= bounds.origin.y);
                        assert!(editor.origin.y + editor.size.height <= footer.origin.y + 0.1);
                        assert!(
                            (footer.origin.y + footer.size.height - bounds.origin.y - height).abs()
                                < 0.1
                        );
                        let text = nodes
                            .iter()
                            .find(|n| n.label.as_deref() == Some("Start typing"))
                            .unwrap();
                        assert_eq!(text.bounds, editor);
                        if visible {
                            let field = nodes
                                .iter()
                                .find(|n| n.label.as_deref() == Some("Find"))
                                .unwrap();
                            assert!(
                                field.bounds.origin.y + field.bounds.size.height
                                    <= editor.origin.y + 0.1
                            );
                        } else {
                            assert!(!nodes.iter().any(|n| n.label.as_deref() == Some("Find")));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn find_and_escape_restore_focus_after_repaint_through_the_real_dispatcher() {
        use appcore::viewkit::{event::EventDispatcher, platform::PlatformEvent};
        let view = fixture_view();
        let theme = Theme::LIGHT;
        let bounds = Rect::new(0., 0., 360., 320.);
        let mut dispatcher = EventDispatcher::new();
        let mut text = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text);
        dispatcher.set_accessibility_nodes(&snapshot(&view, bounds, &theme));
        dispatcher.dispatch(&view, bounds, &PlatformEvent::Focused(true), &mut context);
        dispatcher.dispatch(
            &view,
            bounds,
            &PlatformEvent::KeyPressed {
                key: Key::Character('f'),
                modifiers: KeyModifiers::from_bits(KeyModifiers::CONTROL),
            },
            &mut context,
        );
        dispatcher.set_accessibility_nodes(&snapshot(&view, bounds, &theme));
        dispatcher.dispatch(
            &view,
            bounds,
            &PlatformEvent::TextInput {
                text: "alpha".into(),
            },
            &mut context,
        );
        assert_eq!(view.find_field_state.value(), "alpha");
        assert_eq!(view.editor_state.value(), "alpha βeta alpha");
        assert!(view.find_field_state.is_focused());
        assert!(!view.editor_state.is_focused());
        dispatcher.dispatch(
            &view,
            bounds,
            &PlatformEvent::KeyPressed {
                key: Key::Escape,
                modifiers: KeyModifiers::default(),
            },
            &mut context,
        );
        dispatcher.set_accessibility_nodes(&snapshot(&view, bounds, &theme));
        dispatcher.dispatch(
            &view,
            bounds,
            &PlatformEvent::TextInput {
                text: "replacement".into(),
            },
            &mut context,
        );
        assert!(view.editor_state.value().contains("replacement"));
        assert_eq!(view.find_field_state.value(), "alpha");
        assert!(view.editor_state.is_focused());
        assert!(!view.find_field_state.is_focused());
    }

    #[test]
    fn short_settings_window_keeps_done_reachable_and_preserves_document() {
        use appcore::viewkit::platform::PointerButton;
        let view = fixture_view();
        view.settings_visible.set(true);
        let bounds = Rect::new(0.0, 0.0, 360.0, 320.0);
        let theme = Theme::LIGHT;
        let nodes = snapshot(&view, bounds, &theme);
        let done = nodes
            .iter()
            .find(|n| n.label.as_deref() == Some("Done") && n.role == AccessibilityRole::Button)
            .unwrap()
            .bounds;
        assert!(done.origin.y + done.size.height <= theme.layout.top_bar_height);
        let position = Point::new(
            done.origin.x + done.size.width * 0.5,
            done.origin.y + done.size.height * 0.5,
        );
        let mut text = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text);
        for event in [
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            },
            ViewEvent::PointerReleased {
                position,
                button: PointerButton::Primary,
            },
        ] {
            view.handle_event(bounds, &event, &mut context);
        }
        assert!(!view.settings_visible.get());
        assert_eq!(view.editor_state.value(), "alpha βeta alpha");
    }
}
