# Edit

Edit is the built-in plain-text editor for mochiOS.

The application uses ViewKit's reusable multi-line `TextEditor`. It accepts a
document path as its first launch argument, saves UTF-8 documents atomically
with `Ctrl+S`, and preserves an existing UTF-8 BOM. `Ctrl+F` searches the
document, `Ctrl+,` opens editor settings, `Ctrl+W` closes the window, and
`Ctrl+Q` exits the application.

Editor preferences are stored per user under
`/var/config/applications/org.mochios.edit/users`; they are never written into
the user's home directory. Files and Binder still need to connect their file
association launch flow to the existing document-path contract.
