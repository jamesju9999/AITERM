//! The dependency sets features ask for.
//!
//! All profiles share one venv: isolating each would double the disk cost and
//! the bookkeeping for no benefit, since nothing here has conflicting pins.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// API doc scraping (`tools/ApiDocFetcher`).
    ApiDocs,
    /// Document conversion for the formats most people convert.
    DocCore,
    /// Audio transcription — installed on demand, since most users never
    /// convert an audio file and its SpeechRecognition dependency is a
    /// sizeable download nobody else needs. (Not image conversion: images go
    /// through a vision API directly, and their PIL-based metadata fallback
    /// is already covered by doc_core's `markitdown[pdf]` -> pillow chain.)
    DocAudio,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::ApiDocs, Profile::DocCore, Profile::DocAudio];

    /// Directory under `tools/` (dev) or the resource bundle (production).
    pub fn tool_dir(&self) -> &'static str {
        match self {
            Profile::ApiDocs => "ApiDocFetcher",
            Profile::DocCore | Profile::DocAudio => "MarkItDown",
        }
    }

    pub fn requirements_file(&self) -> &'static str {
        match self {
            Profile::ApiDocs | Profile::DocCore => "requirements.txt",
            Profile::DocAudio => "requirements-audio.txt",
        }
    }

    /// Key under which this profile's installed-hash is recorded.
    pub fn marker_key(&self) -> &'static str {
        match self {
            Profile::ApiDocs => "api_docs",
            Profile::DocCore => "doc_core",
            Profile::DocAudio => "doc_audio",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_profile_has_a_distinct_marker_key() {
        let keys: Vec<&str> = Profile::ALL.iter().map(|p| p.marker_key()).collect();
        let mut deduped = keys.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(keys.len(), deduped.len(), "marker keys must be unique");
    }

    #[test]
    fn audio_profile_reads_the_audio_requirements_file() {
        assert_eq!(Profile::DocAudio.requirements_file(), "requirements-audio.txt");
        assert_eq!(Profile::DocCore.requirements_file(), "requirements.txt");
        assert_eq!(Profile::ApiDocs.requirements_file(), "requirements.txt");
    }

    #[test]
    fn doc_profiles_share_the_markitdown_tool_dir() {
        assert_eq!(Profile::DocCore.tool_dir(), "MarkItDown");
        assert_eq!(Profile::DocAudio.tool_dir(), "MarkItDown");
        assert_eq!(Profile::ApiDocs.tool_dir(), "ApiDocFetcher");
    }

    #[test]
    fn all_lists_every_variant() {
        // ALL is a hand-written literal, so unlike the exhaustive matches above,
        // no compiler check catches a variant missing from it — and the tests
        // that iterate ALL would just silently cover one less profile. This
        // match forces whoever adds a variant to come here, and the assert then
        // fails until ALL is updated too.
        fn variant_count(profile: Profile) -> usize {
            match profile {
                Profile::ApiDocs | Profile::DocCore | Profile::DocAudio => 3,
            }
        }
        assert_eq!(Profile::ALL.len(), variant_count(Profile::ApiDocs));
    }

    #[test]
    fn marker_key_matches_the_serialized_form() {
        // These are two representations of one wire format: the marker file keys
        // off marker_key(), while Tauri commands and the frontend type
        // ("api_docs" | "doc_core" | "doc_audio") go through serde. Pin them
        // together so changing either one can't silently split them apart.
        for profile in Profile::ALL {
            let serialized = serde_json::to_string(&profile).unwrap();
            assert_eq!(serialized, format!("\"{}\"", profile.marker_key()));
        }
    }
}
