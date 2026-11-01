# Edit

Edit is the built-in plain-text editor for mochiOS.

The application uses ViewKit's reusable multi-line `TextEditor`. It accepts a
document path as its first launch argument, saves UTF-8 documents atomically
with `Ctrl+S`, and preserves an existing UTF-8 BOM. `Ctrl+F` searches the
document, `Ctrl+,` opens editor settings, `Ctrl+W` closes the window, and
`Ctrl+Q` exits the application. AppCore's `DocumentController` provides Open,
Save, Save As and prevents replacing or closing an edited document without an
explicit Save, Don't Save or Cancel decision.

The editing area uses the standard pane surface. The find bar uses ViewKit's
`Toolbar` and measured field/result layout; the footer uses a shared sidebar
surface and single-line native stack labels, so document status and statistics
do not paint over one another in narrow windows. Editor settings use
`ActionToolbar`, compact settings rows and a scrollable pane. The custom
chrome separators are removed. The editor, document controller and file format
remain responsible for editing, save confirmation and document I/O.

Editor preferences are stored per user under
`/var/config/applications/org.mochios.edit/users`; they are never written into
the user's home directory. Edit accepts document paths at launch and
capability-backed document delivery through AppCore.

Tests cover UTF-8/BOM and permission preservation, atomic preferences, saving
through the keyboard shortcut, find navigation, focus/field geometry, footer
label bounds in both themes and the settings Done action in a short window.
