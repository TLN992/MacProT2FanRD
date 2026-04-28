use std::collections::HashSet;
use std::path::PathBuf;

use crate::error::{Error, Result};

pub struct FanPath {
    pub name: String,
    pub base_path: PathBuf,
}

pub fn find_fan_paths() -> Result<Vec<FanPath>> {
    let fan_glob = "/sys/devices/pci*/*/*/*/APP0001:00/fan*_input";
    let mut seen = HashSet::new();
    let mut input_paths: Vec<PathBuf> = glob::glob(fan_glob)?
        .filter_map(|p| p.ok())
        .filter_map(|p| {
            let canonical = std::fs::canonicalize(&p).ok()?;
            if seen.insert(canonical.clone()) {
                Some(canonical)
            } else {
                None
            }
        })
        .collect();

    if input_paths.is_empty() {
        return Err(Error::NoFan);
    }

    input_paths.sort();

    let fans: Vec<FanPath> = input_paths
        .into_iter()
        .enumerate()
        .filter_map(|(idx, input_path)| {
            // Strip _input to get the base path: .../APP0001:00/fan1
            let file_name = input_path.file_name()?.to_str()?;
            let fan_prefix = file_name.strip_suffix("_input")?;
            let mut base_path = input_path.clone();
            base_path.set_file_name(fan_prefix);

            let name = format!("fan{idx}");
            Some(FanPath { name, base_path })
        })
        .collect();

    if fans.is_empty() {
        return Err(Error::NoFan);
    }

    Ok(fans)
}
