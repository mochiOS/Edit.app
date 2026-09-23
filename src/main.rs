use viewkit::prelude::*;

struct EditApp {
    editor: TextEditorInteractionState,
}

impl App for EditApp {
    type Body = TextEditor;

    fn new() -> Self {
        Self {
            editor: TextEditorInteractionState::new(),
        }
    }

    fn window(&self) -> WindowOptions {
        WindowOptions::new("Untitled — Edit")
            .size(860.0, 640.0)
            .resizable(true)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        TextEditor::with_interaction(self.editor.clone())
            .placeholder("Start typing")
    }
}

fn main() -> Result<(), ViewKitError> {
    run::<EditApp>()
}
