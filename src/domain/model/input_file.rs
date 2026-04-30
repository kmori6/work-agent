#[derive(Debug, Clone, PartialEq)]
pub struct InputFile {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}
