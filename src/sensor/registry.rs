use std::collections::HashSet;
use std::time::Instant;

use crate::config::DegradedConfig;
use crate::sensor::discovery::{scan_for_drivers, scan_hwmon_devices};
use crate::sensor::{HwmonSensor, SensorStatus};

pub struct SensorRegistry {
    sensors: Vec<HwmonSensor>,
    missing_drivers: HashSet<String>,
    degraded_since: Option<Instant>,
    degraded_config: DegradedConfig,
}

impl SensorRegistry {
    pub fn new(degraded_config: DegradedConfig) -> Self {
        let expected: Vec<&str> = degraded_config.expected_drivers.iter().map(|s| s.as_str()).collect();
        let result = scan_hwmon_devices(&expected);

        let degraded_since = if result.missing_drivers.is_empty() {
            eprintln!(
                "Discovered {} sensor(s), all expected drivers present",
                result.sensors.len()
            );
            None
        } else {
            eprintln!(
                "Discovered {} sensor(s), missing expected drivers: {:?} — entering degraded mode",
                result.sensors.len(),
                result.missing_drivers
            );
            Some(Instant::now())
        };

        for sensor in &result.sensors {
            eprintln!("  Found sensor: {} (driver: {})", sensor.name(), sensor.driver());
        }

        Self {
            sensors: result.sensors,
            missing_drivers: result.missing_drivers,
            degraded_since,
            degraded_config,
        }
    }

    pub fn retry_discovery(&mut self) {
        if self.missing_drivers.is_empty() {
            return;
        }

        let result = scan_for_drivers(&self.missing_drivers);

        if !result.sensors.is_empty() {
            for sensor in &result.sensors {
                eprintln!(
                    "  Late discovery: {} (driver: {})",
                    sensor.name(),
                    sensor.driver()
                );
            }
            self.sensors.extend(result.sensors);
        }

        self.missing_drivers = result.missing_drivers;

        if self.missing_drivers.is_empty() {
            eprintln!("All expected drivers now present — exiting degraded mode");
            self.degraded_since = None;
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded_since.is_some()
    }

    pub fn degraded_fan_percent(&self) -> u8 {
        match self.degraded_since {
            Some(since) => {
                if since.elapsed().as_secs() < self.degraded_config.escalation_delay {
                    self.degraded_config.initial_percent
                } else {
                    self.degraded_config.escalated_percent
                }
            }
            None => 0,
        }
    }

    pub fn poll_all(&mut self) -> Vec<(&str, SensorStatus)> {
        self.sensors
            .iter_mut()
            .map(|s| {
                let status = s.read_temp();
                (s.name(), status)
            })
            .collect()
    }

    pub fn sensors(&self) -> &[HwmonSensor] {
        &self.sensors
    }
}
