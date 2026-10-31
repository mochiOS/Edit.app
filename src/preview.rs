//! Plain-text document preview; no UI, markup execution or file access.
use appcore::document::preview::{MAX_TEXT_BYTES, PreviewContent, PreviewRequest};
use std::io;
pub(crate) fn generate(request: PreviewRequest) -> io::Result<PreviewContent> {
    let mime = request.content_type.split(';').next().unwrap_or_default();
    if !(mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/toml" | "application/xml"
        ))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported text document",
        ));
    }
    let bytes = request
        .bytes
        .strip_prefix(b"\xef\xbb\xbf")
        .unwrap_or(&request.bytes);
    if bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "binary document",
        ));
    }
    let mut end = bytes.len().min(MAX_TEXT_BYTES);
    // Only incomplete UTF-8 at the preview boundary may be omitted.
    let text = loop {
        match std::str::from_utf8(&bytes[..end]) {
            Ok(text) => break text.to_owned(),
            Err(error) if end < bytes.len() && error.error_len().is_none() => {
                end = error.valid_up_to()
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "document is not UTF-8",
                ));
            }
        }
    };
    Ok(PreviewContent::Text {
        text,
        truncated: end < bytes.len(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(bytes: Vec<u8>) -> PreviewRequest {
        PreviewRequest {
            bytes,
            extension: "txt".into(),
            content_type: "text/plain".into(),
            max_edge: 1024,
        }
    }
    #[test]
    fn strips_bom_and_preserves_document_text() {
        let PreviewContent::Text { text, truncated } =
            generate(request(b"\xef\xbb\xbfhello\r\nworld".to_vec())).unwrap()
        else {
            panic!()
        };
        assert_eq!(text, "hello\r\nworld");
        assert!(!truncated);
    }
    #[test]
    fn truncates_at_utf8_boundary_and_rejects_binary() {
        let mut bytes = vec![b'a'; MAX_TEXT_BYTES - 1];
        bytes.extend_from_slice("日本語".as_bytes());
        let PreviewContent::Text { text, truncated } = generate(request(bytes)).unwrap() else {
            panic!()
        };
        assert_eq!(text.len(), MAX_TEXT_BYTES - 1);
        assert!(truncated);
        assert!(generate(request(vec![0, 1])).is_err());
        assert!(generate(request(vec![255])).is_err());
    }
}
