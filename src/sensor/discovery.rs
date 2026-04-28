use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::sensor::HwmonSensor;

const HWMON_BASE: &str = "/sys/class/hwmon";

const KNOWN_DRIVERS: &[&str] = &["coretemp", "amdgpu", "applesmc", "nvme"];

const CURATED_APPLESMC_KEYS: &[&str] = &[
    "TC0P", "TC0D", "TG0P", "TG0D", "TG1P", "Tm0P", "Tm1P", "TH0A", "TH0B", "TB0T", "TB1T",
    "Tp0P", "TA0P",
];

pub struct DiscoveryResult {
    pub sensors: Vec<HwmonSensor>,
    pub missing_drivers: HashSet<String>,
}

pub fn scan_hwmon_devices(expected_drivers: &[&str]) -> DiscoveryResult {
    let mut sensors = Vec::new();
    let mut found_drivers: HashSet<String> = HashSet::new();

    // Track how many hwmon devices each driver has, for disambiguation
    let mut amdgpu_index: usize = 0;

    let mut hwmon_dirs = match hwmon_entries() {
        Ok(dirs) => dirs,
        Err(e) => {
            eprintln!("Warning: cannot read {HWMON_BASE}: {e}");
            return DiscoveryResult {
                sensors,
                missing_drivers: expected_drivers.iter().map(|s| s.to_string()).collect(),
            };
        }
    };
    hwmon_dirs.sort();

    for hwmon_path in hwmon_dirs {
        let driver = match read_trimmed(&hwmon_path.join("name")) {
            Some(name) => name,
            None => continue,
        };

        if !KNOWN_DRIVERS.contains(&driver.as_str()) {
            continue;
        }

        found_drivers.insert(driver.clone());

        match driver.as_str() {
            "coretemp" => discover_coretemp(&hwmon_path, &mut sensors),
            "amdgpu" => {
                discover_amdgpu(&hwmon_path, amdgpu_index, &mut sensors);
                amdgpu_index += 1;
            }
            "applesmc" => discover_applesmc(&hwmon_path, &mut sensors),
            "nvme" => discover_nvme(&hwmon_path, &mut sensors),
            _ => {}
        }
    }

    let missing_drivers = expected_drivers
        .iter()
        .filter(|d| !found_drivers.contains(**d))
        .map(|d| d.to_string())
        .collect();

    DiscoveryResult {
        sensors,
        missing_drivers,
    }
}

/// Re-scan only for specific missing drivers, returning newly found sensors.
pub fn scan_for_drivers(wanted: &HashSet<String>) -> DiscoveryResult {
    let mut sensors = Vec::new();
    let mut still_missing = wanted.clone();

    let mut amdgpu_index: usize = 0;

    let mut hwmon_dirs = match hwmon_entries() {
        Ok(dirs) => dirs,
        Err(_) => {
            return DiscoveryResult {
                sensors,
                missing_drivers: still_missing,
            };
        }
    };
    hwmon_dirs.sort();

    for hwmon_path in hwmon_dirs {
        let driver = match read_trimmed(&hwmon_path.join("name")) {
            Some(name) => name,
            None => continue,
        };

        if !wanted.contains(&driver) {
            if driver == "amdgpu" {
                amdgpu_index += 1;
            }
            continue;
        }

        still_missing.remove(&driver);

        match driver.as_str() {
            "coretemp" => discover_coretemp(&hwmon_path, &mut sensors),
            "amdgpu" => {
                discover_amdgpu(&hwmon_path, amdgpu_index, &mut sensors);
                amdgpu_index += 1;
            }
            "applesmc" => discover_applesmc(&hwmon_path, &mut sensors),
            "nvme" => discover_nvme(&hwmon_path, &mut sensors),
            _ => {}
        }
    }

    DiscoveryResult {
        sensors,
        missing_drivers: still_missing,
    }
}

fn discover_coretemp(hwmon_path: &Path, sensors: &mut Vec<HwmonSensor>) {
    for (input_path, label) in temp_inputs_with_labels(hwmon_path) {
        let name = format!("coretemp {label}");
        if let Ok(file) = File::open(&input_path) {
            sensors.push(HwmonSensor::new(name, "coretemp".into(), file));
        }
    }
}

fn discover_amdgpu(hwmon_path: &Path, gpu_index: usize, sensors: &mut Vec<HwmonSensor>) {
    let entries = temp_inputs_with_labels(hwmon_path);

    // Count total amdgpu hwmon dirs to decide whether to disambiguate
    let need_index = gpu_index > 0 || has_multiple_amdgpu();

    for (input_path, label) in entries {
        let name = if need_index {
            format!("amdgpu[{gpu_index}] {label}")
        } else {
            format!("amdgpu {label}")
        };
        if let Ok(file) = File::open(&input_path) {
            sensors.push(HwmonSensor::new(name, "amdgpu".into(), file));
        }
    }
}

fn discover_applesmc(hwmon_path: &Path, sensors: &mut Vec<HwmonSensor>) {
    for (input_path, label) in temp_inputs_with_labels(hwmon_path) {
        if CURATED_APPLESMC_KEYS.contains(&label.as_str()) {
            let name = format!("applesmc {label}");
            if let Ok(file) = File::open(&input_path) {
                sensors.push(HwmonSensor::new(name, "applesmc".into(), file));
            }
        }
    }
}

fn discover_nvme(hwmon_path: &Path, sensors: &mut Vec<HwmonSensor>) {
    for (input_path, label) in temp_inputs_with_labels(hwmon_path) {
        let name = format!("nvme {label}");
        if let Ok(file) = File::open(&input_path) {
            sensors.push(HwmonSensor::new(name, "nvme".into(), file));
        }
    }
}

// --- helpers ---

fn hwmon_entries() -> Result<Vec<PathBuf>, std::io::Error> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(HWMON_BASE)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            entries.push(path);
        }
    }
    Ok(entries)
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Enumerate all temp*_input files in a hwmon dir, paired with their label.
/// Falls back to "temp<N>" if no label file exists.
fn temp_inputs_with_labels(hwmon_path: &Path) -> Vec<(PathBuf, String)> {
    let pattern = format!("{}/temp*_input", hwmon_path.display());
    let Ok(paths) = glob::glob(&pattern) else {
        return Vec::new();
    };

    let mut results: Vec<(PathBuf, String)> = paths
        .filter_map(|p| p.ok())
        .filter_map(|input_path| {
            let file_name = input_path.file_name()?.to_str()?;
            // temp1_input → temp1
            let prefix = file_name.strip_suffix("_input")?;
            let label_path = input_path.with_file_name(format!("{prefix}_label"));
            let label = read_trimmed(&label_path).unwrap_or_else(|| prefix.to_string());
            Some((input_path, label))
        })
        .collect();

    results.sort_by(|a, b| a.0.cmp(&b.0));
    results
}

fn has_multiple_amdgpu() -> bool {
    let Ok(entries) = hwmon_entries() else {
        return false;
    };
    entries
        .iter()
        .filter(|p| read_trimmed(&p.join("name")).as_deref() == Some("amdgpu"))
        .count()
        > 1
}
