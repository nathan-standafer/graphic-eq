//! Test-only: the MDB-Drums dataset (real drums in real mixes, Southall et al. 2017, CC BY-NC-SA 4.0) as training and
//! test material for the drum detector.
//!
//! The dataset is cloned into `labels/audio/mdb-drums` (git-ignored) and its labels converted by `tools/import-mdb.py`
//! (`labels-full_mix/`). Each song comes as a full mix and as separate stems; the drums stem and the sum of the other stems
//! let one song be mixed many ways (`corpus::remix`), which is how 23 songs become a training set.
//!
//! Environment: `GEQ_MDB` (the `MDB Drums` folder), `GEQ_MDB_LABELS` (the converted labels), and `GEQ_REAL_HOLDOUT`: which songs
//! are kept out of training, `test` (the default: the songs the importer put in the test split), `none`, or a comma separated
//! list of name fragments.

use crate::corpus::{self, Label, Set, Song};
use crate::drum_eval::parse_label_file;
use std::path::{Path, PathBuf};

pub struct Track {
    pub name: String,
    pub split: String,
    pub labels: Vec<Label>,
    mix: PathBuf,
    drum_stems: Vec<PathBuf>,
    band_stems: Vec<PathBuf>,
}

pub fn root() -> PathBuf {
    std::env::var_os("GEQ_MDB").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("labels/audio/mdb-drums/MDB Drums"))
}

/// Every song with labels, in name order; empty when the dataset or the converted labels are not there.
pub fn tracks() -> Vec<Track> {
    let root = root();
    let label_dir = std::env::var_os("GEQ_MDB_LABELS").map(PathBuf::from).unwrap_or_else(|| root.parent().unwrap().join("labels-full_mix"));
    let Ok(rd) = std::fs::read_dir(&label_dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.to_str().map_or(false, |s| s.ends_with(".labels.json"))).collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let name = f.file_name().unwrap().to_str().unwrap().trim_end_matches(".labels.json").to_string();
        let lf = parse_label_file(&std::fs::read_to_string(&f).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let dir = root.join("audio/multi-tracks").join(&name);
        let Ok(meta) = std::fs::read_to_string(dir.join(format!("{name}_METADATA.yaml"))) else { continue };
        // the stems are the entries indented four spaces: `filename:` then `instrument:` (the raw tracks below them are deeper)
        let (mut drum_stems, mut band_stems) = (Vec::new(), Vec::new());
        let mut file = None;
        for line in meta.lines() {
            if let Some(v) = line.strip_prefix("    filename: ") {
                file = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("    instrument: ") {
                if let Some(file) = file.take() {
                    let path = dir.join(format!("{name}_STEMS")).join(file);
                    match v.trim() {
                        "drum set" => drum_stems.push(path),
                        i if i.contains("clap") => {} // percussion the labels do not cover: neither drums nor a quiet band
                        _ => band_stems.push(path),
                    }
                }
            }
        }
        out.push(Track { name: name.clone(), split: lf.split, labels: lf.labels, mix: root.join("audio/full_mix").join(format!("{name}_MIX.wav")), drum_stems, band_stems });
    }
    out
}

/// Whether a song is kept out of training (see the module's environment).
pub fn held_out(t: &Track) -> bool {
    match std::env::var("GEQ_REAL_HOLDOUT").as_deref() {
        Ok("none") => false,
        Ok("test") | Err(_) => t.split == "test",
        Ok(list) => list.split(',').any(|n| !n.is_empty() && t.name.contains(n)),
    }
}

fn sum(paths: &[PathBuf]) -> Vec<[f32; 2]> {
    let mut out: Vec<[f32; 2]> = Vec::new();
    for p in paths {
        let a = crate::synth::load_wav(p.to_str().unwrap()).unwrap_or_else(|e| panic!("{e}"));
        if a.len() > out.len() {
            out.resize(a.len(), [0.0; 2]);
        }
        for (o, x) in out.iter_mut().zip(&a) {
            o[0] += x[0];
            o[1] += x[1];
        }
    }
    out
}

impl Track {
    /// The band without the drums (everything but the drum set and the claps), real music with nothing for the detector to find.
    pub fn band(&self) -> Vec<[f32; 2]> {
        sum(&self.band_stems)
    }

    /// The song as released.
    pub fn mix(&self) -> Vec<[f32; 2]> {
        crate::synth::load_wav(self.mix.to_str().unwrap()).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Training songs from this track: the full mix as released, `n` remixes (the band at other levels, with speech, compression
    /// and a noise floor) and the band alone with no labels.
    pub fn training_songs(&self, n: usize) -> Vec<Song> {
        let drums = sum(&self.drum_stems);
        let band = self.band();
        let seed = self.name.bytes().fold(0xC0FFEEu64, |h, b| h.wrapping_mul(0x100_0000_01B3) ^ b as u64);
        let mut out = vec![Song { name: "mdb_mix", set: Set::Dev, audio: self.mix(), labels: self.labels.clone() }];
        for k in 0..n {
            out.push(Song { name: "mdb_remix", set: Set::Dev, audio: corpus::remix(&drums, &band, seed.wrapping_add(k as u64)), labels: self.labels.clone() });
        }
        out.push(Song { name: "mdb_band", set: Set::Dev, audio: corpus::remix(&[], &band, seed.wrapping_add(1000)), labels: Vec::new() });
        out
    }
}
