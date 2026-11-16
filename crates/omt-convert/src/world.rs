//! Whole-world conversion (UMT Converter: input world → output edition →
//! options → convert → save). Builds a brand-new target world folder:
//! settings via [`crate::level`], chunks per enabled dimension via the
//! per-chunk pipeline. The source is only read.

use std::path::{Path, PathBuf};

use omt_world::Edition;

use crate::error::{ConvertError, Result};
use crate::loss::ConversionReport;
use crate::pipeline::{ConvertOptions, apply_conversions, apply_java_conversions, convert_bedrock_chunks, convert_java_chunks};
use crate::proto::ProtoFilter;
use crate::rules::RegistryRules;

/// UMT converter options that apply to latest ⇄ latest conversion.
#[derive(Debug, Clone)]
pub struct WorldOptions {
    /// Overworld / Nether / End.
    pub dims: [bool; 3],
    /// "Skip Partially-Generated Chunks" (UMT default on).
    pub skip_partial: bool,
    /// "Game Mode" override for the output world.
    pub game_mode: Option<i32>,
    /// Output world name (defaults to the source's).
    pub level_name: Option<String>,
}

impl Default for WorldOptions {
    fn default() -> Self {
        Self { dims: [true; 3], skip_partial: true, game_mode: None, level_name: None }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DimSummary {
    pub dim: u8,
    pub converted: usize,
    pub skipped_proto: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Default)]
pub struct WorldReport {
    pub output: PathBuf,
    pub dims: Vec<DimSummary>,
    pub losses: ConversionReport,
}

impl WorldReport {
    pub fn total_converted(&self) -> usize {
        self.dims.iter().map(|d| d.converted).sum()
    }
}

/// Convert `src` (any supported world folder) into a new world at `dst`
/// in edition `to`. `dst` must not exist yet. `progress` receives short
/// stage messages.
pub fn convert_world(src: &Path, dst: &Path, to: Edition, opts: &WorldOptions, progress: &(dyn Fn(String) + Sync)) -> Result<WorldReport> {
    if dst.exists() {
        return Err(ConvertError::Store(format!("{} already exists — pick a new output folder", dst.display())));
    }
    let detected = omt_world::detect(src).map_err(|e| ConvertError::Store(e.to_string()))?;
    let copts = ConvertOptions { filter: ProtoFilter::new(opts.skip_partial), rules: RegistryRules::embedded().clone() };
    let mut report = WorldReport { output: dst.to_path_buf(), ..Default::default() };
    let io = |e: std::io::Error| ConvertError::Store(format!("{}: {e}", dst.display()));
    match (detected, to) {
        (omt_world::Detected::JavaFolder { .. }, Edition::Bedrock) => {
            let java = omt_java::JavaWorld::open(src).map_err(|e| ConvertError::Store(e.to_string()))?;
            let mut settings = crate::level::read_java_settings(src);
            if let Some(n) = &opts.level_name {
                settings.name = Some(n.clone());
            }
            std::fs::create_dir_all(dst).map_err(io)?;
            std::fs::write(dst.join("level.dat"), crate::level::bedrock_level_dat(&settings, opts.game_mode)?).map_err(io)?;
            std::fs::write(dst.join("levelname.txt"), settings.name.clone().unwrap_or_else(|| "Converted World".into())).map_err(io)?;
            let mut target = omt_bedrock::BedrockWorld::open_writable(dst).map_err(|e| ConvertError::Store(e.to_string()))?;
            for dim in 0..3u8 {
                if !opts.dims[dim as usize] {
                    continue;
                }
                progress(format!("Converting {}…", dim_name(dim)));
                let mut outcome = convert_java_chunks(&java, dim, &copts)?;
                let summary = DimSummary { dim, converted: outcome.converted.len(), skipped_proto: outcome.skipped_proto, failed: outcome.failed.len() };
                progress(format!("Writing {} ({} chunks)…", dim_name(dim), summary.converted));
                apply_conversions(&target, dim, &mut outcome)?;
                report.losses.extend(&outcome.merged_report());
                report.dims.push(summary);
            }
            progress("Flushing database…".into());
            omt_world::WorldStore::flush(&mut target).map_err(|e| ConvertError::Store(e.to_string()))?;
        }
        (omt_world::Detected::BedrockFolder { level_dat, .. }, Edition::Java) => {
            let source = omt_bedrock::BedrockWorld::open_read_only(src).map_err(|e| ConvertError::Store(e.to_string()))?;
            let mut tag = level_dat.nbt.tag.clone();
            if let Some(n) = &opts.level_name {
                tag.insert("LevelName", omt_nbt::Tag::String(n.clone()));
            }
            crate::level::write_java_level(dst, &tag, omt_versions::registry().java_data_version, opts.game_mode)?;
            for dim in 0..3u8 {
                if !opts.dims[dim as usize] {
                    continue;
                }
                progress(format!("Converting {}…", dim_name(dim)));
                let mut outcome = convert_bedrock_chunks(&source, dim, &copts)?;
                let summary = DimSummary { dim, converted: outcome.converted.len(), skipped_proto: outcome.skipped_proto, failed: outcome.failed.len() };
                progress(format!("Writing {} ({} chunks)…", dim_name(dim), summary.converted));
                apply_java_conversions(dst, dim, &mut outcome, None)?;
                report.losses.extend(&outcome.merged_report());
                report.dims.push(summary);
            }
            if let Some(icon) = ["world_icon.jpeg"].iter().map(|n| src.join(n)).find(|p| p.is_file()) {
                // Java expects icon.png; keep the Bedrock JPEG beside it
                // rather than mislabeling the format.
                let _ = std::fs::copy(icon, dst.join("world_icon.jpeg"));
            }
        }
        (omt_world::Detected::JavaFolder { .. }, Edition::Java) | (omt_world::Detected::BedrockFolder { .. }, Edition::Bedrock) => {
            return Err(ConvertError::Store("source and output edition are the same — both are already the latest version; nothing to convert".into()));
        }
        (omt_world::Detected::Archive(p), _) => {
            return Err(ConvertError::Store(format!("{} is an archive; open it through a workspace first", p.display())));
        }
    }
    progress("Done.".into());
    Ok(report)
}

fn dim_name(dim: u8) -> &'static str {
    omt_world::Dimension::from_id(dim).map(|d| d.name()).unwrap_or("?")
}
