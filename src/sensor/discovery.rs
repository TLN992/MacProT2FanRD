use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::sensor::HwmonSensor;

const HWMON_BASE: &str = "/sys/class/hwmon";

const KNOWN_DRIVERS: &[&str] = &["coretemp", "amdgpu", "applesmc", "apple-t2-smc", "nvme"];

/// Glob pattern to find the T2 Apple SMC ACPI device (Mac Pro 7,1 etc.)
const T2_SMC_GLOB: &str = "/sys/devices/LNXSYSTM:00/LNXSYBUS:00/PNP0A08:*/device:*/APP0001:00";

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

    // Discover Apple T2 SMC sensors from the ACPI device path (Mac Pro 7,1)
    let t2_sensors = discover_t2_smc();
    if !t2_sensors.is_empty() {
        found_drivers.insert("apple-t2-smc".into());
        sensors.extend(t2_sensors);
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

    if wanted.contains("apple-t2-smc") {
        let t2_sensors = discover_t2_smc();
        if !t2_sensors.is_empty() {
            still_missing.remove("apple-t2-smc");
            sensors.extend(t2_sensors);
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
        let name = format!("applesmc {label}");
        if let Ok(file) = File::open(&input_path) {
            sensors.push(HwmonSensor::new(name, "applesmc".into(), file));
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

/// Discover Apple T2 SMC sensors from the ACPI device path.
/// On Mac Pro 7,1 (and similar T2 Macs), the SMC sensors are not exposed
/// through the standard hwmon interface but directly under APP0001:00.
fn discover_t2_smc() -> Vec<HwmonSensor> {
    let mut sensors = Vec::new();

    let Ok(paths) = glob::glob(T2_SMC_GLOB) else {
        return sensors;
    };

    for device_path in paths.filter_map(|p| p.ok()) {
        let mut entries = temp_inputs_with_labels(&device_path);
        // Filter out invalid readings (-127°C = sensor not connected)
        entries.retain(|(path, _)| {
            read_trimmed(path)
                .and_then(|v| v.parse::<i32>().ok())
                .map_or(false, |millideg| millideg > -100_000)
        });

        for (input_path, label) in entries {
            let desc = smc_key_description(&label);
            let name = if desc != "Unknown" {
                format!("smc {desc} [{label}]")
            } else {
                format!("smc {label}")
            };
            if let Ok(file) = File::open(&input_path) {
                sensors.push(HwmonSensor::new(name, "apple-t2-smc".into(), file));
            }
        }
    }

    sensors
}

/// Map Apple SMC 4-char temperature key to a human-readable description.
/// Based on the iSMC project (github.com/dkorunic/iSMC) key database and
/// Apple's naming convention for Intel/T2 Macs (Mac Pro 7,1).
fn smc_key_description(key: &str) -> &'static str {
    match key {
        // ── CPU ──
        "TC0P" | "TC0p" => "CPU Proximity 1",
        "TC1P" | "TC1p" => "CPU Proximity 2",
        "TC2P" | "TC2p" => "CPU Proximity 3",
        "TC3P" | "TC3p" => "CPU Proximity 4",
        "TC0D" => "CPU Die 1",
        "TC0E" => "CPU Die Virtual 1",
        "TC0F" => "CPU Die Filtered 1",
        "TC0T" => "CPU Die Alt 1",
        "TC0J" => "CPU Junction 1",
        "TC1F" => "CPU Die Filtered 2",
        "TCXC" | "TCXc" => "CPU PECI",
        "TCXR" | "TCXr" => "CPU PECI Filtered",
        "TCSA" => "CPU System Agent",
        "TCSC" | "TCSc" => "CPU System Cluster",
        "TCGC" | "TCGc" => "CPU Intel Graphics",
        "TCHP" => "CPU Heatpipe",
        "TCS0" => "CPU Cluster 0",
        "TCS1" => "CPU Cluster 1",
        "TCS2" => "CPU Cluster 2",
        "TCS3" => "CPU Cluster 3",

        // ── CPU Junction (Xeon die thermal zones) ──
        "TJ0D" => "Xeon Die Zone 0",
        "TJ0E" => "Xeon Die Zone 0 Virtual",
        "TJ0F" => "Xeon Die Zone 0 Filtered",
        "TJ0P" | "TJ0p" => "Xeon Proximity Zone 0",
        "TJ0T" => "Xeon Die Zone 0 Alt",
        "TJ0J" => "Xeon Junction Zone 0",
        "TJ0s" => "Xeon Status Zone 0",
        "TJ0d" => "Xeon Diode Zone 0",
        "TJ1D" => "Xeon Die Zone 1",
        "TJ1E" => "Xeon Die Zone 1 Virtual",
        "TJ1F" => "Xeon Die Zone 1 Filtered",
        "TJ1P" | "TJ1p" => "Xeon Proximity Zone 1",
        "TJ1T" => "Xeon Die Zone 1 Alt",
        "TJ1J" => "Xeon Junction Zone 1",
        "TJ1s" => "Xeon Status Zone 1",
        "TJ1d" => "Xeon Diode Zone 1",
        "TJ2P" | "TJ2p" => "Xeon Proximity Zone 2",
        "TJ2d" => "Xeon Diode Zone 2",
        "TJ2s" => "Xeon Status Zone 2",
        "TJ3P" | "TJ3p" => "Xeon Proximity Zone 3",
        "TJ3J" => "Xeon Junction Zone 3",
        "TJ4P" | "TJ4p" => "Xeon Proximity Zone 4",
        "TJ4J" => "Xeon Junction Zone 4",
        "TJ5p" => "Xeon Proximity Zone 5",
        "TJ6P" | "TJ6p" => "Xeon Proximity Zone 6",
        "TJtd" => "Xeon Max",
        "TJxd" => "Xeon Peak",

        // ── GPU (2 GPUs in MPX bays, zones 2-6 are thermal aggregates) ──
        "TG0D" => "GPU 1 Die",
        "TG0E" => "GPU 1 Die Virtual",
        "TG0F" => "GPU 1 Die Filtered",
        "TG0P" | "TG0p" => "GPU 1 Proximity",
        "TG0T" => "GPU 1 Die Alt",
        "TG0J" => "GPU 1 Junction",
        "TG0d" => "GPU 1 Diode",
        "TG0s" => "GPU 1 Status",
        "TG1D" => "GPU 2 Die",
        "TG1E" => "GPU 2 Die Virtual",
        "TG1F" => "GPU 2 Die Filtered",
        "TG1P" | "TG1p" => "GPU 2 Proximity",
        "TG1T" => "GPU 2 Die Alt",
        "TG1J" => "GPU 2 Junction",
        "TG1d" => "GPU 2 Diode",
        "TG1s" => "GPU 2 Status",
        "TG2P" | "TG2p" => "GPU Thermal Zone 3",
        "TG2d" => "GPU Thermal Zone 3 Diode",
        "TG2s" => "GPU Thermal Zone 3 Status",
        "TG3P" | "TG3p" => "GPU Thermal Zone 4",
        "TG3J" => "GPU Thermal Zone 4 Junction",
        "TG4P" | "TG4p" => "GPU Thermal Zone 5",
        "TG4J" => "GPU Thermal Zone 5 Junction",
        "TG5p" => "GPU Thermal Zone 6",
        "TG6P" | "TG6p" => "GPU Thermal Zone 7",
        "TGtd" => "GPU Max",
        "TGxd" => "GPU Peak",

        // ── PCIe Slot (TS = slot, per-slot sub-sensors a-j + V) ──
        "TS0V" => "PCIe Slot 0 VRM",
        "TS1V" => "MPX Bay 1 VRM",
        "TS1a" => "MPX Bay 1 Sensor A",
        "TS1b" => "MPX Bay 1 Sensor B",
        "TS1c" => "MPX Bay 1 Sensor C",
        "TS1d" => "MPX Bay 1 Diode",
        "TS1e" => "MPX Bay 1 GPU Proximity",
        "TS1f" => "MPX Bay 1 Sensor F",
        "TS1g" => "MPX Bay 1 Sensor G",
        "TS1h" => "MPX Bay 1 Hotspot",
        "TS1i" => "MPX Bay 1 Inlet",
        "TS1j" => "MPX Bay 1 Sensor J",
        "TS2V" => "MPX Bay 2 VRM",
        "TS2a" => "MPX Bay 2 Sensor A",
        "TS2b" => "MPX Bay 2 Sensor B",
        "TS2c" => "MPX Bay 2 Sensor C",
        "TS2d" => "MPX Bay 2 Diode",
        "TS2e" => "MPX Bay 2 GPU Proximity",
        "TS2f" => "MPX Bay 2 Sensor F",
        "TS2g" => "MPX Bay 2 Sensor G",
        "TS2h" => "MPX Bay 2 Hotspot",
        "TS2i" => "MPX Bay 2 Inlet",
        "TS2j" => "MPX Bay 2 Sensor J",
        "TS3V" => "PCIe Slot 3 VRM",
        "TS4V" => "PCIe Slot 4 VRM",
        "TS4a" => "PCIe Slot 4 Sensor A",
        "TS4b" => "PCIe Slot 4 Sensor B",

        // ── PCIe slot diodes (Te = PCIe, 6 physical slots) ──
        "Te0d" => "PCIe Slot 1 Diode",
        "Te1d" => "PCIe Slot 2 Diode",
        "Te2d" => "PCIe Slot 3 Diode",
        "Te3d" => "PCIe Slot 4 Diode",
        "Te4d" => "PCIe Slot 5 Diode",
        "Te5d" => "PCIe Slot 6 Diode",
        "TexD" => "PCIe Max",
        "Texd" => "PCIe Peak",

        // ── Memory (TM = DIMM proximity, Tm = memory bank) ──
        "TM0P" | "TM0p" => "DIMM Proximity 1",
        "TM0V" => "DIMM VRM",
        "TM1P" | "TM1p" => "DIMM Proximity 2",
        "TM2P" | "TM2p" => "DIMM Proximity 3",
        "TM3P" | "TM3p" => "DIMM Proximity 4",
        "TM4P" | "TM4p" => "DIMM Proximity 5",
        "TM4a" => "DIMM 5 Ambient",
        "TM5P" | "TM5p" => "DIMM Proximity 6",
        "TM6P" | "TM6p" => "DIMM Proximity 7",
        "TM6a" => "DIMM 7 Ambient",
        "TM7P" | "TM7p" => "DIMM Proximity 8",
        "TM8P" | "TM8p" => "DIMM Proximity 9",
        "TMCa" => "Memory Controller",
        "TMVR" => "Memory VRM",
        "TMWP" | "TMWp" => "DIMM Group W",
        "TMXP" | "TMXp" => "DIMM Group X",
        "TMYP" | "TMYp" => "DIMM Group Y",
        "TMZP" | "TMZp" => "DIMM Group Z",
        "Tm0P" | "Tm0p" => "Memory Bank 1",
        "Tm1P" | "Tm1p" => "Memory Bank 2",
        "Tm2P" | "Tm2p" => "Memory Bank 3",
        "Tm3P" | "Tm3p" => "Memory Bank 4",
        "Tm4P" | "Tm4p" => "Memory Bank 5",
        "Tm5P" | "Tm5p" => "Memory Bank 6",
        "Tm6p" => "Memory Bank 7",
        "Tm7P" | "Tm7p" => "Memory Bank 8",
        "Tm8P" | "Tm8p" => "Memory Bank 9",
        "Tm9P" | "Tm9p" => "Memory Bank 10",

        // ── Drive Bay / NVMe (TH) ──
        "TH0P" | "TH0p" => "Drive Bay Proximity",
        "TH0F" => "Drive Bay Front",
        "TH0Q" | "TH0q" => "Drive Bay Sensor 1",
        "TH0X" | "TH0x" => "Drive Bay Sensor 2",
        "TH0a" => "NVMe A",
        "TH0b" => "NVMe B",
        "TH0c" => "NVMe C",
        "TH0d" => "NVMe D",

        // ── Power Supply (Tp, 3 PSUs on Mac Pro 7,1) ──
        "Tp0P" | "Tp0p" => "PSU 1 Proximity",
        "Tp0F" => "PSU 1 Filtered",
        "Tp1P" | "Tp1p" => "PSU 2 Proximity",
        "Tp1F" => "PSU 2 Filtered",
        "Tp2P" | "Tp2p" => "PSU 3 Proximity",
        "Tp2F" => "PSU 3 Filtered",

        // ── Ambient / Airflow ──
        "TA0P" | "TA0p" => "Ambient Inlet",
        "TA0V" => "Ambient Air",

        // ── Board ──
        "TB0p" => "Board Proximity 1",
        "TB1p" => "Board Proximity 2",
        "TBtd" => "Board Max",

        // ── Thunderbolt / IO ──
        "TI0d" => "Thunderbolt 1 Diode",
        "TI1d" => "Thunderbolt 2 Diode",

        // ── PCH ──
        "TPCD" => "PCH Die",

        // ── Fan ──
        "TF0p" => "Fan Zone Proximity",
        "TFxd" => "Fan Zone Peak",

        _ => "Unknown",
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

/// Enumerate all temp*_input files in a dir, paired with their label.
/// Falls back to "temp<N>" if no label file exists.
fn temp_inputs_with_labels(dir: &Path) -> Vec<(PathBuf, String)> {
    let pattern = format!("{}/temp*_input", dir.display());
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
