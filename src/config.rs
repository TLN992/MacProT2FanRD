use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::fan::discovery::FanPath;
use crate::sensor::HwmonSensor;

// ── Speed curve (shared between config and controller) ──

#[derive(Clone, Copy, Debug)]
pub enum SpeedCurve {
    Linear,
    Exponential,
    Logarithmic,
}

impl Default for SpeedCurve {
    fn default() -> Self {
        Self::Linear
    }
}

impl<'de> Deserialize<'de> for SpeedCurve {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "linear" => Ok(Self::Linear),
            "exponential" => Ok(Self::Exponential),
            "logarithmic" => Ok(Self::Logarithmic),
            other => Err(serde::de::Error::custom(format!(
                "unknown speed_curve \"{other}\", expected \"linear\", \"exponential\", or \"logarithmic\""
            ))),
        }
    }
}

impl std::fmt::Display for SpeedCurve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Linear => write!(f, "linear"),
            Self::Exponential => write!(f, "exponential"),
            Self::Logarithmic => write!(f, "logarithmic"),
        }
    }
}

// ── Sensor aggregation ──

#[derive(Clone, Copy, Debug)]
pub enum SensorAggregation {
    Max,
    Average,
}

impl Default for SensorAggregation {
    fn default() -> Self {
        Self::Max
    }
}

impl<'de> Deserialize<'de> for SensorAggregation {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "max" => Ok(Self::Max),
            "average" => Ok(Self::Average),
            other => Err(serde::de::Error::custom(format!(
                "unknown sensor_aggregation \"{other}\", expected \"max\", \"average\", or \"min\""
            ))),
        }
    }
}

impl std::fmt::Display for SensorAggregation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Max => write!(f, "max"),
            Self::Average => write!(f, "average"),
        }
    }
}

// ── Raw (parsed) config structs ──

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct RawConfig {
    pub defaults: DefaultsConfig,
    pub fan: HashMap<String, RawFanConfig>,
    pub degraded: DegradedConfig,
}

impl Default for RawConfig {
    fn default() -> Self {
        Self {
            defaults: DefaultsConfig::default(),
            fan: HashMap::new(),
            degraded: DegradedConfig::default(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct DefaultsConfig {
    pub low_temp: u8,
    pub high_temp: u8,
    pub speed_curve: SpeedCurve,
    pub always_full_speed: bool,
    pub sensor_aggregation: SensorAggregation,
    pub ramp_down_rate: f32,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            low_temp: 55,
            high_temp: 75,
            speed_curve: SpeedCurve::Linear,
            always_full_speed: false,
            sensor_aggregation: SensorAggregation::Max,
            ramp_down_rate: 1.0,
        }
    }
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
pub struct RawFanConfig {
    pub low_temp: Option<u8>,
    pub high_temp: Option<u8>,
    pub speed_curve: Option<SpeedCurve>,
    pub sensor_aggregation: Option<SensorAggregation>,
    pub ramp_down_rate: Option<f32>,
    pub sensors: Option<Vec<String>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct DegradedConfig {
    pub expected_drivers: Vec<String>,
    pub initial_percent: u8,
    pub escalated_percent: u8,
    pub escalation_delay: u64,
}

impl Default for DegradedConfig {
    fn default() -> Self {
        Self {
            expected_drivers: vec!["coretemp".into(), "amdgpu".into(), "apple-t2-smc".into()],
            initial_percent: 60,
            escalated_percent: 80,
            escalation_delay: 60,
        }
    }
}

// ── Resolved config (ready for runtime) ──

#[derive(Clone, Debug)]
pub struct ResolvedFanConfig {
    pub low_temp: u8,
    pub high_temp: u8,
    pub speed_curve: SpeedCurve,
    pub sensor_aggregation: SensorAggregation,
    pub ramp_down_rate: f32,
    pub sensor_indices: Option<Vec<usize>>,
}

// ── Config file path ──

#[cfg(debug_assertions)]
const DEFAULT_CONFIG_PATH: &str = "./macprot2fans.toml";
#[cfg(not(debug_assertions))]
const DEFAULT_CONFIG_PATH: &str = "/etc/macprot2fans.toml";

pub fn config_path(cli_override: Option<&str>) -> PathBuf {
    match cli_override {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(DEFAULT_CONFIG_PATH),
    }
}

// ── Config loading ──

pub fn load_config(path: &Path) -> Result<RawConfig> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let config: RawConfig =
                toml::from_str(&content).map_err(|e| Error::ConfigParse(e.to_string()))?;
            Ok(config)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "Config file not found at {}, generating defaults...",
                path.display()
            );
            let config = RawConfig::default();
            write_default_config(path);
            Ok(config)
        }
        Err(e) => Err(Error::ConfigRead(e)),
    }
}

fn write_default_config(path: &Path) {
    let content = generate_config_toml(&[], &[]);
    match std::fs::write(path, &content) {
        Ok(()) => eprintln!("Default config written to {}", path.display()),
        Err(e) => eprintln!(
            "Warning: could not write default config to {}: {e}",
            path.display()
        ),
    }
}

// ── Config resolution ──

pub fn resolve_config(
    raw: &RawConfig,
    fan_paths: &[FanPath],
    sensors: &[HwmonSensor],
) -> Vec<ResolvedFanConfig> {
    let sensor_names: Vec<&str> = sensors.iter().map(|s| s.name()).collect();

    // Warn about fan sections referencing undiscovered fans
    let discovered_fan_names: Vec<&str> = fan_paths.iter().map(|fp| fp.name.as_str()).collect();
    for fan_name in raw.fan.keys() {
        if !discovered_fan_names.contains(&fan_name.as_str()) {
            eprintln!("Warning: config has [fan.{fan_name}] but no such fan was discovered");
        }
    }

    fan_paths
        .iter()
        .map(|fp| {
            let raw_fan = raw.fan.get(&fp.name);
            resolve_single_fan(&raw.defaults, raw_fan, &sensor_names)
        })
        .collect()
}

fn resolve_single_fan(
    defaults: &DefaultsConfig,
    raw_fan: Option<&RawFanConfig>,
    sensor_names: &[&str],
) -> ResolvedFanConfig {
    let (low_temp, high_temp, speed_curve, sensor_aggregation, ramp_down_rate) = match raw_fan {
        Some(rf) => (
            rf.low_temp.unwrap_or(defaults.low_temp),
            rf.high_temp.unwrap_or(defaults.high_temp),
            rf.speed_curve.unwrap_or(defaults.speed_curve),
            rf.sensor_aggregation.unwrap_or(defaults.sensor_aggregation),
            rf.ramp_down_rate.unwrap_or(defaults.ramp_down_rate),
        ),
        None => (
            defaults.low_temp,
            defaults.high_temp,
            defaults.speed_curve,
            defaults.sensor_aggregation,
            defaults.ramp_down_rate,
        ),
    };

    let sensor_indices = raw_fan.and_then(|rf| {
        rf.sensors
            .as_ref()
            .map(|names| validate_sensor_names(names, sensor_names))
    });

    ResolvedFanConfig {
        low_temp,
        high_temp,
        speed_curve,
        sensor_aggregation,
        ramp_down_rate,
        sensor_indices,
    }
}

// ── Sensor name validation (task 3.3) ──

fn validate_sensor_names(config_names: &[String], discovered: &[&str]) -> Vec<usize> {
    let mut indices = Vec::new();
    for name in config_names {
        match discovered.iter().position(|d| *d == name.as_str()) {
            Some(idx) => indices.push(idx),
            None => {
                eprintln!("Warning: sensor \"{name}\" in config not found in discovered sensors")
            }
        }
    }
    indices
}

// ── Generate config template (tasks 4.1, 4.2) ──

pub fn generate_config_toml(sensors: &[HwmonSensor], fan_paths: &[FanPath]) -> String {
    let mut out = String::new();

    out.push_str("# macprot2fans configuration file\n");
    out.push_str("# See --generate-config for a template with discovered hardware\n\n");

    out.push_str("[defaults]\n");
    out.push_str("low_temp = 55\n");
    out.push_str("high_temp = 75\n");
    out.push_str("speed_curve = \"linear\"  # \"linear\", \"exponential\", or \"logarithmic\"\n");
    out.push_str("always_full_speed = false\n");
    out.push_str("sensor_aggregation = \"max\"  # \"max\", \"average\", or \"min\"\n");
    out.push_str("ramp_down_rate = 1.0  # °C/sec gradual fan ramp-down rate\n\n");

    if sensors.is_empty() && fan_paths.is_empty() {
        // Minimal template without discovered hardware
        out.push_str("# [fan.fan0]\n");
        out.push_str("# low_temp = 55       # override defaults for this fan\n");
        out.push_str("# high_temp = 75\n");
        out.push_str("# speed_curve = \"linear\"\n");
        out.push_str("# always_full_speed = false\n");
        out.push_str("# sensor_aggregation = \"max\"\n");
        out.push_str("# ramp_down_rate = 1.0\n");
        out.push_str("# sensors = []        # list sensor names; omit to use max(all sensors)\n\n");
    } else {
        // Available sensor names as comments
        if !sensors.is_empty() {
            out.push_str("# Available sensors:\n");
            for s in sensors {
                out.push_str(&format!("#   \"{}\"\n", s.name()));
            }
            out.push('\n');
        }

        // Per-fan sections
        for fp in fan_paths {
            out.push_str(&format!("[fan.{}]\n", fp.name));
            out.push_str("# low_temp = 55\n");
            out.push_str("# high_temp = 75\n");
            out.push_str("# speed_curve = \"linear\"\n");
            out.push_str("# always_full_speed = false\n");
            out.push_str("# sensor_aggregation = \"max\"\n");
            out.push_str("# ramp_down_rate = 1.0\n");
            out.push_str("# sensors = []\n\n");
        }
    }

    out.push_str("[degraded]\n");
    out.push_str("expected_drivers = [\"coretemp\", \"amdgpu\", \"apple-t2-smc\"]\n");
    out.push_str("initial_percent = 60\n");
    out.push_str("escalated_percent = 80\n");
    out.push_str("escalation_delay = 60  # seconds before escalation\n");

    out
}

// ── CLI argument parsing ──

pub struct CliArgs {
    pub config_path: Option<String>,
    pub generate_config: bool,
    pub generate_nix: bool,
    pub list_sensors: bool,
    pub list_fans: bool,
    pub status: bool,
}

pub fn parse_cli_args() -> CliArgs {
    let args: Vec<String> = std::env::args().collect();
    let mut cli = CliArgs {
        config_path: None,
        generate_config: false,
        generate_nix: false,
        list_sensors: false,
        list_fans: false,
        status: false,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                i += 1;
                if i < args.len() {
                    cli.config_path = Some(args[i].clone());
                }
            }
            "--generate-config" => cli.generate_config = true,
            "--generate-nix" => cli.generate_nix = true,
            "--list-sensors" => cli.list_sensors = true,
            "--list-fans" => cli.list_fans = true,
            "--status" => cli.status = true,
            _ => {}
        }
        i += 1;
    }

    cli
}
