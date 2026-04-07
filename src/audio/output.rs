#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Aac,
    Mp3,
}

impl OutputFormat {
    pub fn from_trigger(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "aac" => Some(Self::Aac),
            "mp3" => Some(Self::Mp3),
            _ => None,
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Aac => "audio/aac",
            Self::Mp3 => "audio/mpeg",
        }
    }

    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::Mp3 => "mp3",
        }
    }
}
