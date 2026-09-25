mod preferences;

use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use preferences::EditorPreferences;
use viewkit::draw_command::DrawCommand;
use viewkit::event::{EventContext, EventResult, ViewEvent};
use viewkit::platform::PointerButton;
use viewkit::prelude::*;
use viewkit::view::{Constraints, MeasureContext, PaintContext};

struct EditApp {
    editor: TextEditorInteractionState,
    document: Rc<RefCell<DocumentIdentity>>,
    preferences: EditorPreferenceModel,
    settings_visible: State<bool>,
    saved_revision: Rc<Cell<u64>>,
    save_status: Rc<RefCell<Option<String>>>,
}

impl App for EditApp {
    type Body = EditView;

    fn new() -> Self {
        let mut document = DocumentIdentity::from_arguments();
        let editor = TextEditorInteractionState::new();
        if let Some(contents) = document.contents.take() {
            editor.set_value(contents);
        }
        let preferences = EditorPreferenceModel::load();
        Self {
            saved_revision: Rc::new(Cell::new(editor.revision())),
            editor,
            document: Rc::new(RefCell::new(document)),
            preferences,
            settings_visible: State::new(false),
            save_status: Rc::new(RefCell::new(None)),
        }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new(format!("{} — Edit", self.document.borrow().display_name))
            .size(860.0, 640.0)
            .resizable(true)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        EditView::new(
            self.editor.clone(),
            Rc::clone(&self.document),
            self.preferences.clone(),
            self.settings_visible.clone(),
            self.saved_revision.clone(),
            self.save_status.clone(),
        )
    }
}

#[derive(Clone)]
struct DocumentIdentity {
    display_name: String,
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
                display_name: "Untitled".into(),
                file_type: "Plain Text",
                encoding: "UTF-8",
                contents: None,
                path: None,
                writes_bom: false,
                can_save: false,
            };
        };

        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .to_owned();
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
            display_name,
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
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "document has no parent directory",
        )
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid document name"))?;
    let temporary = parent.join(format!(".{name}.edit-{}.new", std::process::id()));
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if writes_bom {
            file.write_all(&[0xef, 0xbb, 0xbf])?;
        }
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(
                &temporary,
                fs::Permissions::from_mode(metadata.permissions().mode() & 0o777),
            )?;
        }
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[derive(Clone)]
struct SaveAsEntry {
    path: PathBuf,
    name: String,
    directory: bool,
}

fn save_as_entries(directory: &Path, root: &Path) -> Vec<SaveAsEntry> {
    let mut entries = fs::read_dir(directory)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if metadata.is_dir()
                && fs::canonicalize(&path)
                    .ok()
                    .is_none_or(|canonical| !canonical.starts_with(root))
            {
                return None;
            }
            Some(SaveAsEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path,
                directory: metadata.is_dir(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .directory
            .cmp(&left.directory)
            .then_with(|| left.name.to_ascii_lowercase().cmp(&right.name.to_ascii_lowercase()))
    });
    entries
}

fn valid_save_name(name: &str) -> bool {
    let path = Path::new(name);
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.bytes().any(|byte| byte == b'/' || byte == 0)
        && path.components().count() == 1
}

fn attempt_save_as(
    document: &Rc<RefCell<DocumentIdentity>>,
    editor: &TextEditorInteractionState,
    directory: &Rc<RefCell<PathBuf>>,
    name: &TextFieldInteractionState,
    pending_replace: &Rc<RefCell<Option<PathBuf>>>,
    error: &Rc<RefCell<Option<String>>>,
    saved_revision: &Rc<Cell<u64>>,
    save_status: &Rc<RefCell<Option<String>>>,
    visible: &State<bool>,
) {
    let filename = name.value();
    if !valid_save_name(filename.trim()) {
        *error.borrow_mut() = Some(String::from("Enter a valid file name."));
        return;
    }
    let Ok(directory) = fs::canonicalize(directory.borrow().as_path()) else {
        *error.borrow_mut() = Some(String::from("The selected folder is no longer available."));
        return;
    };
    let destination = directory.join(filename.trim());
    if destination.is_dir() {
        *error.borrow_mut() = Some(String::from("A folder already uses this name."));
        return;
    }
    if destination.exists() && pending_replace.borrow().as_ref() != Some(&destination) {
        *pending_replace.borrow_mut() = Some(destination);
        *error.borrow_mut() = Some(String::from(
            "A file already uses this name. Select Save again to replace it.",
        ));
        return;
    }
    let writes_bom = document.borrow().writes_bom;
    match save_document(&destination, &editor.value(), writes_bom) {
        Ok(()) => {
            let mut identity = document.borrow_mut();
            identity.display_name = filename.trim().to_owned();
            identity.file_type = file_type_for_path(&destination);
            identity.encoding = if writes_bom { "UTF-8 with BOM" } else { "UTF-8" };
            identity.path = Some(destination);
            identity.can_save = true;
            saved_revision.set(editor.revision());
            *save_status.borrow_mut() = Some(String::from("Saved"));
            error.borrow_mut().take();
            pending_replace.borrow_mut().take();
            visible.set(false);
        }
        Err(save_error) => {
            *error.borrow_mut() = Some(format!("Unable to save: {save_error}"));
        }
    }
}

struct SaveAsPanel {
    visible: State<bool>,
    root: PathBuf,
    directory: Rc<RefCell<PathBuf>>,
    entries: Rc<RefCell<Vec<SaveAsEntry>>>,
    selected: Rc<Cell<Option<usize>>>,
    scroll: Rc<Cell<f32>>,
    last_click: Rc<RefCell<Option<(usize, Instant)>>>,
    name_state: TextFieldInteractionState,
    name_field: TextField,
    error: Rc<RefCell<Option<String>>>,
    pending_replace: Rc<RefCell<Option<PathBuf>>>,
    up: Button,
    cancel: Button,
    save: Button,
}

#[derive(Clone, Copy)]
struct SaveAsGeometry {
    dialog: Rect,
    up: Rect,
    location: Rect,
    list: Rect,
    name: Rect,
    error: Rect,
    cancel: Rect,
    save: Rect,
}

impl SaveAsPanel {
    fn new(
        document: Rc<RefCell<DocumentIdentity>>,
        editor: TextEditorInteractionState,
        saved_revision: Rc<Cell<u64>>,
        save_status: Rc<RefCell<Option<String>>>,
    ) -> Self {
        let root = std::env::var_os("HOME")
            .map(PathBuf::from)
            .and_then(|path| fs::canonicalize(path).ok())
            .unwrap_or_else(|| PathBuf::from("/home/testuser"));
        let initial = document
            .borrow()
            .path
            .as_deref()
            .and_then(Path::parent)
            .and_then(|path| fs::canonicalize(path).ok())
            .filter(|path| path.starts_with(&root))
            .unwrap_or_else(|| root.clone());
        let directory = Rc::new(RefCell::new(initial.clone()));
        let entries = Rc::new(RefCell::new(save_as_entries(&initial, &root)));
        let visible = State::new(false);
        let name_state = TextFieldInteractionState::new();
        let initial_name = document.borrow().display_name.clone();
        name_state.set_value(if initial_name == "Untitled" {
            String::from("Untitled.txt")
        } else {
            initial_name
        });
        let name_field = TextField::with_interaction(name_state.clone()).placeholder("File name");
        let selected = Rc::new(Cell::new(None));
        let scroll = Rc::new(Cell::new(0.0));
        let error = Rc::new(RefCell::new(None));
        let pending_replace = Rc::new(RefCell::new(None));

        let up_directory = Rc::clone(&directory);
        let up_entries = Rc::clone(&entries);
        let up_selected = Rc::clone(&selected);
        let up_scroll = Rc::clone(&scroll);
        let up_root = root.clone();
        let up = Button::new("Up")
            .size(ButtonSize::Small)
            .style(ButtonStyle::Ghost)
            .on_click(move || {
                let parent = up_directory
                    .borrow()
                    .parent()
                    .map(Path::to_path_buf)
                    .filter(|path| path.starts_with(&up_root));
                if let Some(parent) = parent {
                    *up_directory.borrow_mut() = parent.clone();
                    *up_entries.borrow_mut() = save_as_entries(&parent, &up_root);
                    up_selected.set(None);
                    up_scroll.set(0.0);
                }
            });

        let cancel_visible = visible.clone();
        let cancel_error = Rc::clone(&error);
        let cancel_pending = Rc::clone(&pending_replace);
        let cancel = Button::new("Cancel")
            .size(ButtonSize::Small)
            .style(ButtonStyle::Standard)
            .on_click(move || {
                cancel_visible.set(false);
                cancel_error.borrow_mut().take();
                cancel_pending.borrow_mut().take();
            });

        let save_document_identity = Rc::clone(&document);
        let save_editor = editor.clone();
        let save_directory = Rc::clone(&directory);
        let save_name = name_state.clone();
        let save_pending = Rc::clone(&pending_replace);
        let save_error = Rc::clone(&error);
        let save_revision = Rc::clone(&saved_revision);
        let save_status_state = Rc::clone(&save_status);
        let save_visible = visible.clone();
        let save = Button::new("Save")
            .size(ButtonSize::Small)
            .style(ButtonStyle::Accent)
            .on_click(move || {
                attempt_save_as(
                    &save_document_identity,
                    &save_editor,
                    &save_directory,
                    &save_name,
                    &save_pending,
                    &save_error,
                    &save_revision,
                    &save_status_state,
                    &save_visible,
                );
            });

        Self {
            visible,
            root,
            directory,
            entries,
            selected,
            scroll,
            last_click: Rc::new(RefCell::new(None)),
            name_state,
            name_field,
            error,
            pending_replace,
            up,
            cancel,
            save,
        }
    }

    fn is_visible(&self) -> bool {
        self.visible.get()
    }

    fn show(&self) {
        self.error.borrow_mut().take();
        self.pending_replace.borrow_mut().take();
        self.name_state.set_focused(true);
        self.visible.set(true);
    }

    fn geometry(bounds: Rect) -> SaveAsGeometry {
        let width = (bounds.size.width - 64.0).clamp(420.0, 680.0);
        let height = (bounds.size.height - 64.0).clamp(360.0, 520.0);
        let dialog = Rect::new(
            bounds.origin.x + (bounds.size.width - width) / 2.0,
            bounds.origin.y + (bounds.size.height - height) / 2.0,
            width,
            height,
        );
        let inset = 20.0;
        let top = dialog.origin.y + 54.0;
        let up = Rect::new(dialog.origin.x + inset, top, 56.0, 30.0);
        let location = Rect::new(dialog.origin.x + 88.0, top, width - 108.0, 30.0);
        let list = Rect::new(
            dialog.origin.x + inset,
            top + 42.0,
            width - inset * 2.0,
            height - 190.0,
        );
        let name = Rect::new(dialog.origin.x + inset, dialog.origin.y + height - 86.0, width - 236.0, 32.0);
        let error = Rect::new(dialog.origin.x + inset, dialog.origin.y + height - 48.0, width - 220.0, 22.0);
        let cancel = Rect::new(dialog.origin.x + width - 188.0, dialog.origin.y + height - 86.0, 76.0, 32.0);
        let save = Rect::new(dialog.origin.x + width - 100.0, dialog.origin.y + height - 86.0, 80.0, 32.0);
        SaveAsGeometry { dialog, up, location, list, name, error, cancel, save }
    }

    fn navigate(&self, path: PathBuf) {
        let Ok(path) = fs::canonicalize(path) else { return; };
        if !path.starts_with(&self.root) || !path.is_dir() {
            return;
        }
        *self.directory.borrow_mut() = path.clone();
        *self.entries.borrow_mut() = save_as_entries(&path, &self.root);
        self.selected.set(None);
        self.scroll.set(0.0);
        self.last_click.borrow_mut().take();
    }
}

impl View for SaveAsPanel {
    fn measure(&self, constraints: Constraints, _context: &mut MeasureContext<'_>) -> Size {
        constraints.constrain(constraints.maximum)
    }

    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        let geometry = Self::geometry(bounds);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.shell.scrim))
            .paint(bounds, context);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.dialog.background))
            .radius(context.theme.dialog.radius)
            .border(BorderStyle::custom(
                context.theme.dialog.border,
                context.theme.dialog.stroke_width,
            ))
            .paint(geometry.dialog, context);
        Text::styled("Save As", TextRole::TitleSmall).paint(
            Rect::new(geometry.dialog.origin.x + 20.0, geometry.dialog.origin.y + 18.0, geometry.dialog.size.width - 40.0, 26.0),
            context,
        );
        self.up.paint(geometry.up, context);
        Text::body(self.directory.borrow().display().to_string()).paint(geometry.location, context);
        Rectangle::new()
            .color(RectangleColor::Custom(context.theme.colors.surface_subtle))
            .radius(CornerRadius::Small)
            .border(BorderStyle::custom(context.theme.colors.border, 1.0))
            .paint(geometry.list, context);
        context.display_list.push(DrawCommand::PushClip { rect: geometry.list });
        let row_height = 34.0;
        for (index, entry) in self.entries.borrow().iter().enumerate() {
            let row = Rect::new(
                geometry.list.origin.x,
                geometry.list.origin.y + index as f32 * row_height - self.scroll.get(),
                geometry.list.size.width,
                row_height,
            );
            if row.origin.y + row.size.height <= geometry.list.origin.y
                || row.origin.y >= geometry.list.origin.y + geometry.list.size.height
            {
                continue;
            }
            if self.selected.get() == Some(index) {
                Rectangle::new()
                    .color(RectangleColor::Custom(context.theme.colors.accent_soft))
                    .paint(row, context);
            }
            let label = if entry.directory {
                format!("{} /", entry.name)
            } else {
                entry.name.clone()
            };
            Text::body(label).paint(
                Rect::new(row.origin.x + 12.0, row.origin.y + 7.0, row.size.width - 24.0, 20.0),
                context,
            );
        }
        context.display_list.push(DrawCommand::PopClip);
        self.name_field.paint(geometry.name, context);
        if let Some(error) = self.error.borrow().as_ref() {
            Text::metadata(error.clone()).paint(geometry.error, context);
        }
        self.cancel.paint(geometry.cancel, context);
        self.save.paint(geometry.save, context);
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        let geometry = Self::geometry(bounds);
        if matches!(event, ViewEvent::KeyPressed { key: Key::Escape, .. }) {
            self.visible.set(false);
            self.name_state.set_focused(false);
            context.request_redraw();
            return EventResult::Consumed;
        }
        let previous_name = self.name_state.value();
        let mut result = self.name_field.handle_event(geometry.name, event, context);
        if self.name_state.value() != previous_name {
            self.pending_replace.borrow_mut().take();
            self.error.borrow_mut().take();
        }
        result = result
            .merge(self.up.handle_event(geometry.up, event, context))
            .merge(self.cancel.handle_event(geometry.cancel, event, context))
            .merge(self.save.handle_event(geometry.save, event, context));
        if let ViewEvent::Scroll { position, delta_y, .. } = event
            && geometry.list.contains(*position)
        {
            let maximum = (self.entries.borrow().len() as f32 * 34.0 - geometry.list.size.height).max(0.0);
            self.scroll.set((self.scroll.get() - *delta_y * 34.0).clamp(0.0, maximum));
            context.request_redraw_in(geometry.list);
            return EventResult::Consumed;
        }
        if let ViewEvent::PointerReleased { position, button: PointerButton::Primary } = event
            && geometry.list.contains(*position)
        {
            let index = ((position.y - geometry.list.origin.y + self.scroll.get()) / 34.0) as usize;
            if let Some(entry) = self.entries.borrow().get(index).cloned() {
                let now = Instant::now();
                let double_click = self.last_click.borrow().as_ref().is_some_and(|(previous, instant)| {
                    *previous == index && now.saturating_duration_since(*instant) <= Duration::from_millis(500)
                });
                *self.last_click.borrow_mut() = Some((index, now));
                self.selected.set(Some(index));
                if entry.directory && double_click {
                    self.navigate(entry.path);
                } else if !entry.directory {
                    self.name_state.set_value(entry.name);
                    self.pending_replace.borrow_mut().take();
                }
                context.request_redraw();
            }
            return EventResult::Consumed;
        }
        if result.is_consumed() { result } else { EventResult::Consumed }
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
    document: Rc<RefCell<DocumentIdentity>>,
    preferences: EditorPreferenceModel,
    settings_visible: State<bool>,
    settings: EditSettingsView,
    saved_revision: Rc<Cell<u64>>,
    save_status: Rc<RefCell<Option<String>>>,
    save_as: SaveAsPanel,
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
        document: Rc<RefCell<DocumentIdentity>>,
        preferences: EditorPreferenceModel,
        settings_visible: State<bool>,
        saved_revision: Rc<Cell<u64>>,
        save_status: Rc<RefCell<Option<String>>>,
    ) -> Self {
        let find_field_state = TextFieldInteractionState::new();
        let settings = EditSettingsView::new(preferences.clone(), settings_visible.clone());
        let save_as = SaveAsPanel::new(
            Rc::clone(&document),
            editor_state.clone(),
            Rc::clone(&saved_revision),
            Rc::clone(&save_status),
        );
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
            find_state: RefCell::new(FindState::default()),
            document,
            preferences,
            settings_visible,
            settings,
            saved_revision,
            save_status,
            save_as,
        }
    }

    fn save(&self) -> bool {
        let document = self.document.borrow();
        let Some(path) = document.path.as_deref() else {
            drop(document);
            self.save_as.show();
            return false;
        };
        if !document.can_save {
            *self.save_status.borrow_mut() = Some(String::from("This document cannot be saved"));
            return false;
        }
        match save_document(path, &self.editor_state.value(), document.writes_bom) {
            Ok(()) => {
                self.saved_revision.set(self.editor_state.revision());
                *self.save_status.borrow_mut() = Some(String::from("Saved"));
                true
            }
            Err(error) => {
                *self.save_status.borrow_mut() = Some(format!("Unable to save: {error}"));
                false
            }
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
        let document = self.document.borrow();
        let mut file_status = document.file_type.to_owned();
        if let Some(status) = self.save_status.borrow().as_ref() {
            file_status.push_str("    ");
            file_status.push_str(status);
        } else if self.editor_state.revision() != self.saved_revision.get() {
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
                document.encoding, lines, line_label, characters, character_label
            )
        } else {
            format!(
                "{}    {} {}",
                document.encoding, characters, character_label
            )
        };
        Text::metadata(statistics)
            .alignment(TextAlignment::End)
            .paint(footer_text, context);
        drop(document);
        if self.save_as.is_visible() {
            self.save_as.paint(bounds, context);
        }
    }

    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if self.save_as.is_visible() {
            return self.save_as.handle_event(bounds, event, context);
        }

        if matches!(
            event,
            ViewEvent::KeyPressed {
                key: Key::Character('q' | 'Q' | 'w' | 'W'),
                modifiers
            } if modifiers.shortcut()
        ) {
            request_exit();
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
            self.save_as.show();
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
            self.save_status.borrow_mut().take();
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
    use std::time::{SystemTime, UNIX_EPOCH};
    use viewkit::platform::KeyModifiers;
    use viewkit::typography::TextMeasurer;

    #[test]
    fn recognizes_structured_text_types() {
        assert_eq!(file_type_for_path(Path::new("settings.json")), "JSON");
        assert_eq!(file_type_for_path(Path::new("version.toml")), "TOML");
        assert_eq!(file_type_for_path(Path::new("notes.txt")), "Plain Text");
        assert_eq!(file_type_for_path(Path::new("README")), "Plain Text");
    }

    #[test]
    fn atomic_save_preserves_utf8_bom_and_permissions() {
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
    fn save_as_requires_confirmation_before_replacing_a_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mochios-edit-save-as-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let destination = root.join("notes.txt");
        fs::write(&destination, "old").unwrap();

        let document = Rc::new(RefCell::new(DocumentIdentity {
            display_name: String::from("Untitled"),
            file_type: "Plain Text",
            encoding: "UTF-8",
            contents: None,
            path: None,
            writes_bom: false,
            can_save: false,
        }));
        let editor = TextEditorInteractionState::new();
        editor.set_value("replacement");
        let directory = Rc::new(RefCell::new(root.clone()));
        let name = TextFieldInteractionState::new();
        name.set_value("notes.txt");
        let pending = Rc::new(RefCell::new(None));
        let error = Rc::new(RefCell::new(None));
        let saved_revision = Rc::new(Cell::new(0));
        let status = Rc::new(RefCell::new(None));
        let visible = State::new(true);

        attempt_save_as(
            &document,
            &editor,
            &directory,
            &name,
            &pending,
            &error,
            &saved_revision,
            &status,
            &visible,
        );
        assert_eq!(fs::read_to_string(&destination).unwrap(), "old");
        assert_eq!(pending.borrow().as_ref(), Some(&destination));

        attempt_save_as(
            &document,
            &editor,
            &directory,
            &name,
            &pending,
            &error,
            &saved_revision,
            &status,
            &visible,
        );
        assert_eq!(fs::read_to_string(&destination).unwrap(), "replacement");
        assert_eq!(document.borrow().path.as_ref(), Some(&destination));
        assert!(!visible.get());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_as_rejects_path_components_as_file_names() {
        assert!(valid_save_name("notes.txt"));
        assert!(!valid_save_name("../notes.txt"));
        assert!(!valid_save_name("folder/notes.txt"));
        assert!(!valid_save_name(".."));
    }

    #[test]
    fn save_shortcut_writes_the_current_editor_contents() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("mochios-edit-shortcut-{}-{unique}", std::process::id()));
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
        let view = EditView::new(
            editor,
            Rc::new(RefCell::new(DocumentIdentity {
                display_name: String::from("document.txt"),
                file_type: "Plain Text",
                encoding: "UTF-8",
                contents: None,
                path: Some(path.clone()),
                writes_bom: false,
                can_save: true,
            })),
            preference_model,
            State::new(false),
            Rc::new(Cell::new(0)),
            Rc::new(RefCell::new(None)),
        );
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
