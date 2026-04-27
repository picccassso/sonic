use id3::{frame::Picture, Tag, TagLike, Version};
use std::io::Cursor;

use crate::errors::TranscodeError;

#[derive(Debug, Clone, Default)]
pub struct AudioMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track: Option<u32>,
    pub artwork: Option<Picture>,
}

impl AudioMetadata {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.genre.is_none()
            && self.year.is_none()
            && self.track.is_none()
            && self.artwork.is_none()
    }
}

pub fn extract_mp3_metadata(input: &[u8]) -> Option<AudioMetadata> {
    let mut cursor = Cursor::new(input);
    let tag = Tag::read_from2(&mut cursor).ok()?;
    let artwork = tag.pictures().next().cloned();
    Some(AudioMetadata {
        title: tag.title().map(str::to_owned),
        artist: tag.artist().map(str::to_owned),
        album: tag.album().map(str::to_owned),
        genre: tag.genre().map(str::to_owned),
        year: tag.year(),
        track: tag.track(),
        artwork,
    })
}

pub fn prepend_id3_metadata(
    data: Vec<u8>,
    metadata: &AudioMetadata,
) -> Result<Vec<u8>, TranscodeError> {
    if metadata.is_empty() {
        return Ok(data);
    }

    let mut tag = Tag::new();

    if let Some(value) = &metadata.title {
        tag.set_title(value);
    }
    if let Some(value) = &metadata.artist {
        tag.set_artist(value);
    }
    if let Some(value) = &metadata.album {
        tag.set_album(value);
    }
    if let Some(value) = &metadata.genre {
        tag.set_genre(value);
    }
    if let Some(value) = metadata.year {
        tag.set_year(value);
    }
    if let Some(value) = metadata.track {
        tag.set_track(value);
    }
    if let Some(value) = metadata.artwork.clone() {
        tag.add_frame(value);
    }

    let mut tagged = Vec::new();
    tag.write_to(&mut tagged, Version::Id3v24)
        .map_err(|err| TranscodeError::Encode(format!("failed to write ID3 metadata: {err}")))?;
    tagged.extend_from_slice(&data);
    Ok(tagged)
}

#[cfg(test)]
mod tests {
    use id3::{Tag, TagLike, Version};

    use super::{extract_mp3_metadata, prepend_id3_metadata};

    #[test]
    fn extracts_and_prepends_basic_id3_metadata() {
        let mut input = Vec::new();
        let mut tag = Tag::new();
        tag.set_title("Track");
        tag.set_artist("Artist");
        tag.set_album("Album");
        tag.set_track(7);
        tag.write_to(&mut input, Version::Id3v24)
            .expect("write input tag");
        input.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);

        let metadata = extract_mp3_metadata(&input).expect("extract metadata");
        assert_eq!(metadata.title.as_deref(), Some("Track"));
        assert_eq!(metadata.artist.as_deref(), Some("Artist"));
        assert_eq!(metadata.album.as_deref(), Some("Album"));
        assert_eq!(metadata.track, Some(7));

        let output = prepend_id3_metadata(vec![1, 2, 3], &metadata).expect("prepend metadata");
        assert!(output.starts_with(b"ID3"));
        assert!(output.ends_with(&[1, 2, 3]));
    }
}
