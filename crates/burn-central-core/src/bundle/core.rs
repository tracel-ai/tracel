use serde::{Serialize, de::DeserializeOwned};
use std::io::Read;

/// Trait for encoding data into a bundle of files
pub trait BundleEncode {
    type Settings: Default + Serialize + DeserializeOwned;
    type Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>;

    fn encode<O: BundleSink>(
        self,
        sink: &mut O,
        settings: &Self::Settings,
    ) -> Result<(), Self::Error>;
}

/// Trait for decoding data from a bundle of files
pub trait BundleDecode: Sized {
    type Settings: Default + Serialize + DeserializeOwned;
    type Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>;

    fn decode<I: BundleSource>(source: &I, settings: &Self::Settings) -> Result<Self, Self::Error>;
}

/// Trait for writing files to a bundle
pub trait BundleSink {
    /// Add a file by streaming its bytes. Returns computed checksum + size.
    fn put_file<R: Read>(&mut self, path: &str, reader: &mut R) -> Result<(), String>;

    /// Convenience: write all bytes.
    fn put_bytes(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let mut r = std::io::Cursor::new(bytes);
        self.put_file(path, &mut r)
    }
}

/// Trait for reading files from a bundle
pub trait BundleSource {
    /// Open the given path for streaming read. Must validate existence.
    fn open(&self, path: &str) -> Result<Box<dyn Read + Send>, String>;

    /// Optionally list available files (used by generic decoders; can be best-effort).
    fn list(&self) -> Result<Vec<String>, String>;
}

/// Normalize a path within a bundle (use forward slashes, remove leading slash)
pub fn normalize_bundle_path<S: AsRef<str>>(s: S) -> String {
    s.as_ref()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}
