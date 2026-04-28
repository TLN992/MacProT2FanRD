mod config;
mod error;
mod fan;
mod sensor;
mod wizard;

use std::collections::VecDeque;
use std::io::ErrorKind;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use signal_hook::consts::{SIGINT, SIGTERM};

use config::{
    DegradedConfig, ResolvedFanConfig, SensorAggregation, config_path, load_config, parse_cli_args,
    resolve_config,
};
use error::{Error, Result};
use fan::FanController;
use fan::discovery::find_fan_paths;
use sensor::SensorStatus;
use sensor::registry::SensorRegistry;

#[cfg(not(target_os = "linux"))]
compile_error!("This tool is only developed for Linux systems.");

#[cfg(debug_assertions)]
const PID_FILE: &str = "macprot2fans.pid";
#[cfg(not(debug_assertions))]
const PID_FILE: &str = "/run/macprot2fans.pid";

const RETRY_INTERVAL: Duration = Duration::from_secs(3);

fn main() -> ExitCode {
    let cli = parse_cli_args();

    if cli.list_sensors {
        list_sensors_and_exit();
        return ExitCode::SUCCESS;
    }

    if cli.list_fans {
        list_fans_and_exit();
        return ExitCode::SUCCESS;
    }

    if cli.generate_config {
        run_wizard_and_exit(false);
        return ExitCode::SUCCESS;
    }

    if cli.generate_nix {
        run_wizard_and_exit(true);
        return ExitCode::SUCCESS;
    }

    match run_daemon(cli.config_path.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn get_current_euid() -> u32 {
    // SAFETY: FFI call with no preconditions
    unsafe { libc::geteuid() }
}

fn check_pid_file() -> Result<()> {
    match std::fs::read_to_string(PID_FILE) {
        Ok(pid) => {
            let proc_path = std::path::PathBuf::from(format!("/proc/{}", pid.trim()));
            if proc_path.exists() {
                return Err(Error::AlreadyRunning);
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(Error::PidRead(err)),
    }

    let current_pid = std::process::id().to_string();
    std::fs::write(PID_FILE, current_pid).map_err(Error::PidWrite)
}

fn run_daemon(config_override: Option<&str>) -> Result<()> {
    if get_current_euid() != 0 {
        return Err(Error::NotRoot);
    }

    check_pid_file()?;

    // Load config
    let path = config_path(config_override);
    let raw_config = load_config(&path)?;

    // Discover hardware
    let fan_paths = find_fan_paths()?;
    let mut registry = SensorRegistry::new(raw_config.degraded.clone());

    // Resolve per-fan configs
    let resolved = resolve_config(&raw_config, &fan_paths, registry.sensors());

    let mut fans: Vec<FanController> = fan_paths
        .into_iter()
        .zip(resolved)
        .map(|(fp, cfg)| FanController::new(fp, cfg))
        .collect::<Result<_>>()?;

    for fan in &mut fans {
        fan.open_control_files()?;
    }

    for fan in &fans {
        fan.set_manual(true)?;
    }

    let res = run_temp_loop(&mut fans, &mut registry);

    eprintln!("Fan daemon is shutting down...");
    for fan in &fans {
        let _ = fan.set_manual(false);
    }

    let pid_res = std::fs::remove_file(PID_FILE).map_err(Error::PidDelete);

    match (res, pid_res) {
        (Err(err), _) | (_, Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn aggregate_temps(temps: &[u8], method: SensorAggregation) -> Option<u8> {
    if temps.is_empty() {
        return None;
    }
    match method {
        SensorAggregation::Max => temps.iter().copied().max(),
        SensorAggregation::Average => {
            let sum: u32 = temps.iter().map(|&t| u32::from(t)).sum();
            Some((sum / temps.len() as u32) as u8)
        }
    }
}

#[derive(Clone)]
struct TempBuffer(VecDeque<f32>);

impl TempBuffer {
    pub fn new() -> Self {
        TempBuffer(VecDeque::with_capacity(16))
    }

    pub fn push(&mut self, value: f32) {
        self.0.push_back(value);
        while self.0.len() > 16 {
            self.0.pop_front();
        }
    }

    pub fn temp(&self) -> f32 {
        self.0.iter().sum::<f32>() / self.0.len() as f32
    }
}

fn run_temp_loop(fans: &mut [FanController], registry: &mut SensorRegistry) -> Result<()> {
    let cancellation_token = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, cancellation_token.clone()).map_err(Error::Signal)?;
    signal_hook::flag::register(SIGTERM, cancellation_token.clone()).map_err(Error::Signal)?;

    let fan_count = fans.len();
    let mut effective_temps: Vec<TempBuffer> = vec![TempBuffer::new(); fan_count];
    let mut per_fan_last_speed: Vec<u32> = vec![0; fan_count];
    let mut last_cycle = Instant::now();
    let mut last_retry = Instant::now();
    let mut last_status = Instant::now();
    const STATUS_INTERVAL: Duration = Duration::from_secs(10);

    while !cancellation_token.load(Ordering::Relaxed) {
        if registry.is_degraded() {
            let percent = registry.degraded_fan_percent();
            for fan in fans.iter() {
                fan.set_speed_percent(percent)?;
            }

            if last_retry.elapsed() >= RETRY_INTERVAL {
                registry.retry_discovery();
                last_retry = Instant::now();
            }

            std::thread::sleep(RETRY_INTERVAL);
            last_cycle = Instant::now();
            continue;
        }

        let elapsed_secs = last_cycle.elapsed().as_secs_f32();
        last_cycle = Instant::now();

        let readings = registry.poll_all();

        // Collect all active temps for global fallback
        let all_active: Vec<u8> = readings
            .iter()
            .filter_map(|(_, status)| match status {
                SensorStatus::Active(t) => Some(*t),
                _ => None,
            })
            .collect();

        let mut any_changed = false;
        for (i, fan) in fans.iter().enumerate() {
            // Gather active temps for this fan's sensors
            let fan_temps: Vec<u8> = match fan.sensor_indices() {
                Some(indices) if !indices.is_empty() => indices
                    .iter()
                    .filter_map(|&idx| {
                        readings.get(idx).and_then(|(_, status)| match status {
                            SensorStatus::Active(t) => Some(*t),
                            _ => None,
                        })
                    })
                    .collect(),
                _ => all_active.clone(),
            };

            let raw_temp =
                aggregate_temps(&fan_temps, fan.sensor_aggregation()).unwrap_or(0) as f32;

            // Asymmetric ramp: instant up, gradual down
            if raw_temp >= effective_temps[i].temp() {
                effective_temps[i].push(raw_temp);
            } else {
                let decay = fan.ramp_down_rate() * elapsed_secs;
                let temp = effective_temps[i].temp();
                effective_temps[i].push((temp - decay).max(raw_temp));
            }

            let speed = fan.calc_speed(effective_temps[i].temp() as u8);
            if speed != per_fan_last_speed[i] {
                per_fan_last_speed[i] = speed;
                fan.set_speed(speed)?;
                any_changed = true;
            }
        }

        // Periodic status output
        if last_status.elapsed() >= STATUS_INTERVAL {
            last_status = Instant::now();
            eprintln!("\n");
            for (i, fan) in fans.iter_mut().enumerate() {
                let rpm = fan.read_rpm().map_or("ERR".into(), |r| format!("{r}"));
                eprintln!(
                    "  {} | temp={:.0}°C | RPM={}",
                    fan.name(),
                    effective_temps[i].temp(),
                    rpm,
                );
            }
        }

        if any_changed {
            std::thread::sleep(Duration::from_millis(100));
        } else {
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    Ok(())
}

fn list_sensors_and_exit() {
    let mut registry = SensorRegistry::new(DegradedConfig::default());
    let readings = registry.poll_all();

    if readings.is_empty() {
        println!("No sensors found.");
        return;
    }

    println!("{:<30} {:>8}", "SENSOR", "TEMP");
    println!("{:-<30} {:->8}", "", "");

    for (name, status) in readings {
        let temp_str = match status {
            SensorStatus::Active(t) => format!("{t}°C"),
            SensorStatus::Unavailable => "N/A".into(),
            SensorStatus::Error(e) => format!("ERR: {e}"),
        };
        println!("{:<30} {:>8}", name, temp_str);
    }
}

fn list_fans_and_exit() {
    let fan_paths = match find_fan_paths() {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let default_config = ResolvedFanConfig {
        low_temp: 55,
        high_temp: 75,
        speed_curve: config::SpeedCurve::Linear,
        sensor_aggregation: config::SensorAggregation::Max,
        ramp_down_rate: 1.0,
        sensor_indices: None,
    };

    let mut fans: Vec<FanController> = fan_paths
        .into_iter()
        .filter_map(|fp| match FanController::new(fp, default_config.clone()) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("Warning: {e}");
                None
            }
        })
        .collect();

    if fans.is_empty() {
        eprintln!("No fans found.");
        std::process::exit(1);
    }

    println!(
        "{:<10} {:>10} {:>10} {:>12}",
        "FAN", "MIN RPM", "MAX RPM", "CURRENT RPM"
    );
    println!("{:-<10} {:->10} {:->10} {:->12}", "", "", "", "");

    for fan in &mut fans {
        let current = fan.read_rpm().map_or("ERR".into(), |r| format!("{r}"));
        println!(
            "{:<10} {:>10} {:>10} {:>12}",
            fan.name(),
            fan.min_speed(),
            fan.max_speed(),
            current
        );
    }
}

fn run_wizard_and_exit(nix_format: bool) {
    let registry = SensorRegistry::new(DegradedConfig::default());
    let fan_paths = match find_fan_paths() {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("Warning: could not discover fans: {e}");
            Vec::new()
        }
    };

    let config = wizard::run_wizard(registry.sensors(), &fan_paths);

    if nix_format {
        print!("{}", wizard::format_nix(&config));
    } else {
        print!("{}", wizard::format_toml(&config));
    }
}
