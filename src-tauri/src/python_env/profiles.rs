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
    /// Image and audio conversion — installed on demand, since these extras
    /// are the bulk of the download.
    DocMedia,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::ApiDocs, Profile::DocCore, Profile::DocMedia];

    /// Directory under `tools/` (dev) or the resource bundle (production).
    pub fn tool_dir(&self) -> &'static str {
        match self {
            Profile::ApiDocs => "ApiDocFetcher",
            Profile::DocCore | Profile::DocMedia => "MarkItDown",
        }
    }

    pub fn requirements_file(&self) -> &'static str {
        match self {
            Profile::ApiDocs | Profile::DocCore => "requirements.txt",
            Profile::DocMedia => "requirements-media.txt",
        }
    }

    /// Key under which this profile's installed-hash is recorded.
    pub fn marker_key(&self) -> &'static str {
        match self {
            Profile::ApiDocs => "api_docs",
            Profile::DocCore => "doc_core",
            Profile::DocMedia => "doc_media",
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
    fn media_profile_reads_the_media_requirements_file() {
        assert_eq!(Profile::DocMedia.requirements_file(), "requirements-media.txt");
        assert_eq!(Profile::DocCore.requirements_file(), "requirements.txt");
        assert_eq!(Profile::ApiDocs.requirements_file(), "requirements.txt");
    }

    #[test]
    fn doc_profiles_share_the_markitdown_tool_dir() {
        assert_eq!(Profile::DocCore.tool_dir(), "MarkItDown");
        assert_eq!(Profile::DocMedia.tool_dir(), "MarkItDown");
        assert_eq!(Profile::ApiDocs.tool_dir(), "ApiDocFetcher");
    }
}
