use viewkit::prelude::*;

struct EditApp {
    editor: TextEditorInteractionState,
    revision: State<u64>,
}

impl App for EditApp {
    type Body = VStack;

    fn new() -> Self {
        Self {
            editor: TextEditorInteractionState::new(),
            revision: State::new(0),
        }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new("Untitled — Edit")
            .size(860.0, 640.0)
            .resizable(true)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        let _ = self.revision.get();
        let undo_editor = self.editor.clone();
        let undo_revision = self.revision.clone();
        let redo_editor = self.editor.clone();
        let redo_revision = self.revision.clone();
        let edit_revision = self.revision.clone();

        VStack::new()
            .alignment(StackAlignment::Stretch)
            .gap(StackGap::None)
            .child(
                Padding::symmetric(
                    Theme::current().spacing.large,
                    Theme::current().spacing.small,
                )
                .content(
                    HStack::new()
                        .alignment(StackAlignment::Center)
                        .gap(StackGap::Small)
                        .child(
                            Button::new("Undo")
                                .style(ButtonStyle::Ghost)
                                .size(ButtonSize::Small)
                                .enabled(self.editor.can_undo())
                                .on_click(move || {
                                    if undo_editor.perform_undo() {
                                        undo_revision.update(|revision| {
                                            *revision = revision.wrapping_add(1);
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("Redo")
                                .style(ButtonStyle::Ghost)
                                .size(ButtonSize::Small)
                                .enabled(self.editor.can_redo())
                                .on_click(move || {
                                    if redo_editor.perform_redo() {
                                        redo_revision.update(|revision| {
                                            *revision = revision.wrapping_add(1);
                                        });
                                    }
                                }),
                        )
                        .child(Spacer::new())
                        .child(Text::metadata("Plain Text")),
                ),
            )
            .child(Divider::new())
            .child(
                TextEditor::with_interaction(self.editor.clone())
                    .placeholder("Start typing")
                    .on_change(move |_| {
                        edit_revision.update(|revision| {
                            *revision = revision.wrapping_add(1);
                        });
                    })
                    .layout()
                    .flex_grow(1.0),
            )
    }
}

fn main() -> Result<(), ViewKitError> {
    run::<EditApp>()
}
