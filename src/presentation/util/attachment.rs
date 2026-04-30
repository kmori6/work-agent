use crate::application::usecase::agent_usecase::Attachment;
use crate::domain::model::input_file::InputFile;
use crate::domain::model::input_image::InputImage;
use std::path::Path;

pub fn load_attachment(path: &Path) -> Result<Attachment, String> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let mime_type = mime_type_from_path(path);
    let data = std::fs::read(path).map_err(|e| format!("cannot read file: {e}"))?;

    if mime_type.starts_with("image/") {
        Ok(Attachment::Image(InputImage { mime_type, data }))
    } else {
        Ok(Attachment::File(InputFile {
            filename,
            mime_type,
            data,
        }))
    }
}

pub fn mime_type_from_path(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("csv") => "text/csv",
        Some("txt") => "text/plain",
        Some("md") => "text/markdown",
        Some("html") => "text/html",
        _ => "application/octet-stream",
    }
    .to_string()
}
