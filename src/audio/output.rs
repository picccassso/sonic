#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Aac,
    M4a,
    Mp3,
    Opus,
}

impl OutputFormat {
    pub fn from_trigger(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "aac" => Some(Self::Aac),
            "m4a" | "mp4" => Some(Self::M4a),
            "mp3" => Some(Self::Mp3),
            "opus" | "ogg" => Some(Self::Opus),
            _ => None,
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Aac => "audio/aac",
            Self::M4a => "audio/mp4",
            Self::Mp3 => "audio/mpeg",
            Self::Opus => "audio/ogg; codecs=opus",
        }
    }

    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
        }
    }
}
