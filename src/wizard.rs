use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use crate::config::{
    DefaultsConfig, DegradedConfig, RawConfig, RawFanConfig, SensorAggregation, SpeedCurve,
};
use crate::fan::discovery::FanPath;
use crate::sensor::HwmonSensor;

fn prompt_line(prompt: &str, default: &str) -> String {
    eprint!("{prompt} [{default}]: ");
    io::stderr().flush().unwrap();
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).unwrap();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn prompt_select(prompt: &str, options: &[&str], default_idx: usize) -> usize {
    eprintln!("{prompt}");
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == default_idx { " (default)" } else { "" };
        eprintln!("  {}) {opt}{marker}", i + 1);
    }
    eprint!("Selection [{}]: ", default_idx + 1);
    io::stderr().flush().unwrap();
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input).unwrap();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return default_idx;
    }
    match trimmed.parse::<usize>() {
        Ok(n) if n >= 1 && n <= options.len() => n - 1,
        _ => {
            eprintln!("Invalid selection, using default.");
            default_idx
        }
    }
}

pub fn run_wizard(sensors: &[HwmonSensor], fan_paths: &[FanPath]) -> RawConfig {
    eprintln!("\n=== macprot2fans Configuration Wizard ===\n");
    eprintln!(
        "Discovered {} sensor(s), {} fan(s).\n",
        sensors.len(),
        fan_paths.len()
    );

    // Global defaults
    eprintln!("── Global Defaults ──");
    let low_temp: u8 = prompt_line("Low temp (min RPM)", "55")
        .parse()
        .unwrap_or(55);
    let high_temp: u8 = prompt_line("High temp (max RPM)", "75")
        .parse()
        .unwrap_or(75);

    let curve_opts = &["linear", "exponential", "logarithmic"];
    let curve_idx = prompt_select("Speed curve?", curve_opts, 0);
    let speed_curve = match curve_idx {
        1 => SpeedCurve::Exponential,
        2 => SpeedCurve::Logarithmic,
        _ => SpeedCurve::Linear,
    };

    let agg_opts = &["max", "average", "min"];
    let agg_idx = prompt_select("Sensor aggregation?", agg_opts, 0);
    let sensor_aggregation = match agg_idx {
        1 => SensorAggregation::Average,
        _ => SensorAggregation::Max,
    };

    let ramp_down_rate: f32 = prompt_line("Ramp down rate (°C/sec)", "1.0")
        .parse()
        .unwrap_or(1.0);

    let defaults = DefaultsConfig {
        low_temp,
        high_temp,
        speed_curve,
        always_full_speed: false,
        sensor_aggregation,
        ramp_down_rate,
    };

    // Per-fan configuration
    let mut fan_configs: HashMap<String, RawFanConfig> = HashMap::new();

    for fp in fan_paths {
        eprintln!("\n── {} ──", fp.name);
        let customize = prompt_line("Customize this fan? (y/n)", "n");
        if customize.to_lowercase() != "y" {
            continue;
        }

        let fan_low: Option<u8> = {
            let v = prompt_line("  Low temp override (Enter=inherit)", &low_temp.to_string());
            let parsed = v.parse().ok();
            if parsed == Some(low_temp) {
                None
            } else {
                parsed
            }
        };
        let fan_high: Option<u8> = {
            let v = prompt_line(
                "  High temp override (Enter=inherit)",
                &high_temp.to_string(),
            );
            let parsed = v.parse().ok();
            if parsed == Some(high_temp) {
                None
            } else {
                parsed
            }
        };

        let fan_curve_idx = prompt_select("  Speed curve?", curve_opts, curve_idx);
        let fan_curve = if fan_curve_idx == curve_idx {
            None
        } else {
            Some(match fan_curve_idx {
                1 => SpeedCurve::Exponential,
                2 => SpeedCurve::Logarithmic,
                _ => SpeedCurve::Linear,
            })
        };

        let fan_agg_idx = prompt_select("  Sensor aggregation?", agg_opts, agg_idx);
        let fan_agg = if fan_agg_idx == agg_idx {
            None
        } else {
            Some(match fan_agg_idx {
                1 => SensorAggregation::Average,
                _ => SensorAggregation::Max,
            })
        };

        let fan_ramp: Option<f32> = {
            let v = prompt_line(
                "  Ramp down rate (Enter=inherit)",
                &ramp_down_rate.to_string(),
            );
            let parsed: f32 = v.parse().unwrap_or(ramp_down_rate);
            if (parsed - ramp_down_rate).abs() < 0.001 {
                None
            } else {
                Some(parsed)
            }
        };

        // Sensor selection
        let selected_sensors = if !sensors.is_empty() {
            eprintln!("  Available sensors:");
            for (i, s) in sensors.iter().enumerate() {
                eprintln!("    {}) {}", i + 1, s.name());
            }
            let input = prompt_line("  Sensors (comma-separated numbers, or 'all')", "all");
            if input == "all" {
                None
            } else {
                let names: Vec<String> = input
                    .split(',')
                    .filter_map(|s| {
                        let idx: usize = s.trim().parse().ok()?;
                        sensors
                            .get(idx.checked_sub(1)?)
                            .map(|s| s.name().to_string())
                    })
                    .collect();
                if names.is_empty() { None } else { Some(names) }
            }
        } else {
            None
        };

        fan_configs.insert(
            fp.name.clone(),
            RawFanConfig {
                low_temp: fan_low,
                high_temp: fan_high,
                speed_curve: fan_curve,
                sensor_aggregation: fan_agg,
                ramp_down_rate: fan_ramp,
                sensors: selected_sensors,
            },
        );
    }

    RawConfig {
        defaults,
        fan: fan_configs,
        degraded: DegradedConfig::default(),
    }
}

// ── TOML output ──

pub fn format_toml(config: &RawConfig) -> String {
    let mut out = String::new();
    let d = &config.defaults;

    out.push_str("# macprot2fans configuration file\n\n");

    out.push_str("[defaults]\n");
    out.push_str(&format!("low_temp = {}\n", d.low_temp));
    out.push_str(&format!("high_temp = {}\n", d.high_temp));
    out.push_str(&format!("speed_curve = \"{}\"\n", d.speed_curve));
    out.push_str(&format!("always_full_speed = {}\n", d.always_full_speed));
    out.push_str(&format!(
        "sensor_aggregation = \"{}\"\n",
        d.sensor_aggregation
    ));
    out.push_str(&format!("ramp_down_rate = {}\n", d.ramp_down_rate));
    out.push('\n');

    // Sort fan names for deterministic output
    let mut fan_names: Vec<&String> = config.fan.keys().collect();
    fan_names.sort();

    for name in fan_names {
        let fc = &config.fan[name];
        out.push_str(&format!("[fan.{name}]\n"));
        if let Some(v) = fc.low_temp {
            out.push_str(&format!("low_temp = {v}\n"));
        }
        if let Some(v) = fc.high_temp {
            out.push_str(&format!("high_temp = {v}\n"));
        }
        if let Some(v) = fc.speed_curve {
            out.push_str(&format!("speed_curve = \"{v}\"\n"));
        }
        if let Some(v) = fc.sensor_aggregation {
            out.push_str(&format!("sensor_aggregation = \"{v}\"\n"));
        }
        if let Some(v) = fc.ramp_down_rate {
            out.push_str(&format!("ramp_down_rate = {v}\n"));
        }
        if let Some(ref sensors) = fc.sensors {
            let quoted: Vec<String> = sensors.iter().map(|s| format!("\"{s}\"")).collect();
            out.push_str(&format!("sensors = [{}]\n", quoted.join(", ")));
        }
        out.push('\n');
    }

    let dg = &config.degraded;
    out.push_str("[degraded]\n");
    let drivers: Vec<String> = dg
        .expected_drivers
        .iter()
        .map(|d| format!("\"{d}\""))
        .collect();
    out.push_str(&format!("expected_drivers = [{}]\n", drivers.join(", ")));
    out.push_str(&format!("initial_percent = {}\n", dg.initial_percent));
    out.push_str(&format!("escalated_percent = {}\n", dg.escalated_percent));
    out.push_str(&format!("escalation_delay = {}\n", dg.escalation_delay));

    out
}

// ── Nix output ──

pub fn format_nix(config: &RawConfig) -> String {
    let mut out = String::new();
    let d = &config.defaults;

    out.push_str("{\n");
    out.push_str("  services.macprot2fans = {\n");
    out.push_str("    enable = true;\n\n");

    out.push_str("    defaults = {\n");
    out.push_str(&format!("      low_temp = {};\n", d.low_temp));
    out.push_str(&format!("      high_temp = {};\n", d.high_temp));
    out.push_str(&format!("      speed_curve = \"{}\";\n", d.speed_curve));
    out.push_str(&format!(
        "      always_full_speed = {};\n",
        d.always_full_speed
    ));
    out.push_str(&format!(
        "      sensor_aggregation = \"{}\";\n",
        d.sensor_aggregation
    ));
    out.push_str(&format!(
        "      ramp_down_rate = {};\n",
        format_nix_float(d.ramp_down_rate)
    ));
    out.push_str("    };\n\n");

    if !config.fan.is_empty() {
        out.push_str("    fans = {\n");
        let mut fan_names: Vec<&String> = config.fan.keys().collect();
        fan_names.sort();

        for name in fan_names {
            let fc = &config.fan[name];
            out.push_str(&format!("      {name} = {{\n"));
            if let Some(v) = fc.low_temp {
                out.push_str(&format!("        low_temp = {v};\n"));
            }
            if let Some(v) = fc.high_temp {
                out.push_str(&format!("        high_temp = {v};\n"));
            }
            if let Some(v) = fc.speed_curve {
                out.push_str(&format!("        speed_curve = \"{v}\";\n"));
            }
            if let Some(v) = fc.sensor_aggregation {
                out.push_str(&format!("        sensor_aggregation = \"{v}\";\n"));
            }
            if let Some(v) = fc.ramp_down_rate {
                out.push_str(&format!(
                    "        ramp_down_rate = {};\n",
                    format_nix_float(v)
                ));
            }
            if let Some(ref sensors) = fc.sensors {
                out.push_str("        sensors = [\n");
                for s in sensors {
                    out.push_str(&format!("          \"{s}\"\n"));
                }
                out.push_str("        ];\n");
            }
            out.push_str("      };\n");
        }
        out.push_str("    };\n\n");
    }

    let dg = &config.degraded;
    out.push_str("    degraded = {\n");
    out.push_str("      expected_drivers = [\n");
    for drv in &dg.expected_drivers {
        out.push_str(&format!("        \"{drv}\"\n"));
    }
    out.push_str("      ];\n");
    out.push_str(&format!(
        "      initial_percent = {};\n",
        dg.initial_percent
    ));
    out.push_str(&format!(
        "      escalated_percent = {};\n",
        dg.escalated_percent
    ));
    out.push_str(&format!(
        "      escalation_delay = {};\n",
        dg.escalation_delay
    ));
    out.push_str("    };\n");

    out.push_str("  };\n");
    out.push_str("}\n");

    out
}

fn format_nix_float(v: f32) -> String {
    let s = format!("{v}");
    if s.contains('.') { s } else { format!("{s}.0") }
}
