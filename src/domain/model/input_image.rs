#[derive(Debug, Clone, PartialEq)]
pub struct InputImage {
    pub mime_type: String,
    pub data: Vec<u8>,
}
