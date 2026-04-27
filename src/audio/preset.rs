#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityPreset {
    Low,
    Medium,
    High,
    VeryHigh,
}

impl QualityPreset {
    pub fn bitrate_kbps(self) -> u32 {
        match self {
            Self::Low => 64,
            Self::Medium => 128,
            Self::High => 192,
            Self::VeryHigh => 320,
        }
    }

    pub fn from_trigger(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "LOW" => Some(Self::Low),
            "MEDIUM" => Some(Self::Medium),
            "HIGH" => Some(Self::High),
            "VERY_HIGH" | "VERYHIGH" | "MAX" => Some(Self::VeryHigh),
            _ => None,
        }
    }

    pub fn allowed_values() -> &'static str {
        "LOW, MEDIUM, HIGH, VERY_HIGH"
    }
}

#[cfg(test)]
mod tests {
    use super::QualityPreset;

    #[test]
    fn parses_preset_case_insensitive() {
        assert_eq!(QualityPreset::from_trigger("LOW"), Some(QualityPreset::Low));
        assert_eq!(QualityPreset::from_trigger("low"), Some(QualityPreset::Low));
        assert_eq!(
            QualityPreset::from_trigger("MeDiUm"),
            Some(QualityPreset::Medium)
        );
        assert_eq!(
            QualityPreset::from_trigger("HIGH"),
            Some(QualityPreset::High)
        );
        assert_eq!(
            QualityPreset::from_trigger("max"),
            Some(QualityPreset::VeryHigh)
        );
        assert_eq!(QualityPreset::from_trigger("bad"), None);
    }

    #[test]
    fn maps_bitrate() {
        assert_eq!(QualityPreset::Low.bitrate_kbps(), 64);
        assert_eq!(QualityPreset::Medium.bitrate_kbps(), 128);
        assert_eq!(QualityPreset::High.bitrate_kbps(), 192);
        assert_eq!(QualityPreset::VeryHigh.bitrate_kbps(), 320);
    }
}
