//! Downloads a model version with a live per-file progress bar.
//!
//! [`tracel::console::Models::download`] reports byte-level progress through a
//! [`TransferObserver`], which is exactly what an interactive terminal wants for something as
//! slow as a model download.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cliclack::log;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tracel::console::{FsBundle, ModelVersion, Models, ModelsError, TransferObserver};

/// Downloads `version` of `model` into `out`, rendering one progress bar per file.
pub fn download(
    models: &Models,
    model: &str,
    version: &ModelVersion,
    out: &Path,
) -> anyhow::Result<()> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let handler_flag = Arc::clone(&cancelled);
    ctrlc::set_handler(move || handler_flag.store(true, Ordering::Relaxed))?;

    log::step(format!(
        "Downloading {model} v{} to {}",
        version.version,
        out.display()
    ))?;

    let mut sink = FsBundle::create(out)?;
    let mut observer = DownloadProgress::new(cancelled);

    match models.download(model, &version.id, &mut sink, &mut observer) {
        Ok(()) => {
            log::success(format!("Saved to {}", out.display()))?;
            Ok(())
        }
        Err(ModelsError::Cancelled) => {
            log::warning("Download cancelled")?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Renders each file's transfer as its own bar in a shared [`MultiProgress`].
struct DownloadProgress {
    multi: MultiProgress,
    bars: HashMap<String, ProgressBar>,
    cancelled: Arc<AtomicBool>,
}

impl DownloadProgress {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            multi: MultiProgress::new(),
            bars: HashMap::new(),
            cancelled,
        }
    }

    fn style() -> ProgressStyle {
        ProgressStyle::with_template(
            "{spinner:.cyan} {msg:<32} [{bar:30.cyan/blue}] {bytes}/{total_bytes}",
        )
        .expect("progress template is valid")
        .progress_chars("=>-")
    }
}

impl TransferObserver for DownloadProgress {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn file_started(&mut self, rel_path: &str, total_bytes: Option<u64>) {
        let bar = self.multi.add(ProgressBar::new(total_bytes.unwrap_or(0)));
        bar.set_style(Self::style());
        bar.set_message(rel_path.to_string());
        self.bars.insert(rel_path.to_string(), bar);
    }

    fn file_progress(&mut self, rel_path: &str, transferred_bytes: u64) {
        if let Some(bar) = self.bars.get(rel_path) {
            bar.set_position(transferred_bytes);
        }
    }

    fn file_completed(&mut self, rel_path: &str, transferred_bytes: u64) {
        if let Some(bar) = self.bars.remove(rel_path) {
            bar.set_position(transferred_bytes);
            bar.finish();
        }
    }
}
