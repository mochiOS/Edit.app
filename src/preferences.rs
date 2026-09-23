use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[cfg(target_os = "mochios")]
const STORAGE_ROOT: &str = "/var/config/applications/org.mochios.edit/users";

#[cfg(not(target_os = "mochios"))]
const STORAGE_ROOT: &str = "/tmp/mochios-edit/users";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EditorPreferences {
    pub line_wrap: bool,
    pub show_line_count: bool,
    pub font_size: f32,
}

impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            line_wrap: true,
            show_line_count: true,
            font_size: 15.0,
        }
    }
}

impl EditorPreferences {
    pub(crate) fn load() -> Self {
        let Some(path) = storage_path() else {
            return Self::default();
        };
        recover(&path).ok();
        fs::read_to_string(path)
            .ok()
            .map(|contents| Self::parse(&contents))
            .unwrap_or_default()
    }

    pub(crate) fn save(&self) -> io::Result<()> {
        let path = storage_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid session user name")
        })?;
        self.save_to(&path)
    }

    fn parse(contents: &str) -> Self {
        let mut preferences = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "line_wrap" => preferences.line_wrap = parse_bool(value, true),
                "show_line_count" => preferences.show_line_count = parse_bool(value, true),
                "font_size" => preferences.font_size = parse_font_size(value),
                _ => {}
            }
        }
        preferences
    }

    fn encode(self) -> String {
        format!(
            "format=1\nline_wrap={}\nshow_line_count={}\nfont_size={}\n",
            self.line_wrap, self.show_line_count, self.font_size,
        )
    }

    fn save_to(self, path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid preferences path")
        })?;
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        recover(path)?;

        let id = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".preferences-{}-{id}.new", std::process::id()));
        let backup = parent.join(".preferences.conf.backup");
        remove_if_present(&temporary)?;
        remove_if_present(&backup)?;
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(self.encode().as_bytes())?;
            file.sync_all()?;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
            let had_original = path.exists();
            if had_original {
                fs::rename(path, &backup)?;
            }
            if let Err(error) = fs::rename(&temporary, path) {
                if had_original {
                    let _ = fs::rename(&backup, path);
                }
                return Err(error);
            }
            remove_if_present(&backup)
        })();
        if result.is_err() {
            let _ = remove_if_present(&temporary);
        }
        result
    }
}

fn storage_path() -> Option<PathBuf> {
    let user = std::env::var("USER").unwrap_or_else(|_| String::from("root"));
    valid_user_name(&user).then(|| Path::new(STORAGE_ROOT).join(user).join("preferences.conf"))
}

fn valid_user_name(user: &str) -> bool {
    !user.is_empty()
        && user.len() <= 64
        && user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value.trim() {
        "true" => true,
        "false" => false,
        _ => fallback,
    }
}

fn parse_font_size(value: &str) -> f32 {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(10.0, 32.0))
        .unwrap_or(15.0)
}

fn recover(path: &Path) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let backup = parent.join(".preferences.conf.backup");
    if backup.exists() {
        if path.exists() {
            fs::remove_file(backup)
        } else {
            fs::rename(backup, path)
        }
    } else {
        Ok(())
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_values_are_bounded_or_defaulted() {
        let preferences = EditorPreferences::parse(
            "line_wrap=false\nshow_line_count=no\nfont_size=900\nunknown=true\n",
        );
        assert!(!preferences.line_wrap);
        assert!(preferences.show_line_count);
        assert_eq!(preferences.font_size, 32.0);
    }

    #[test]
    fn preferences_are_replaced_atomically() {
        let root = std::env::temp_dir().join(format!(
            "mochios-edit-preferences-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed),
        ));
        let path = root.join("preferences.conf");
        EditorPreferences {
            line_wrap: false,
            show_line_count: true,
            font_size: 18.0,
        }
        .save_to(&path)
        .unwrap();
        assert_eq!(
            EditorPreferences::parse(&fs::read_to_string(&path).unwrap()),
            EditorPreferences {
                line_wrap: false,
                show_line_count: true,
                font_size: 18.0,
            }
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
