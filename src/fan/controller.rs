use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::PathBuf;

use crate::config::{ResolvedFanConfig, SensorAggregation, SpeedCurve};
use crate::error::{Error, Result};
use crate::fan::discovery::FanPath;

pub struct FanController {
    name: String,
    base_path: PathBuf,
    manual_file: Option<File>,
    output_file: Option<File>,
    input_file: File,
    config: ResolvedFanConfig,
    min_speed: u32,
    max_speed: u32,
}

impl FanController {
    pub fn new(fan_path: FanPath, config: ResolvedFanConfig) -> Result<Self> {
        let base = &fan_path.base_path;

        let min_speed = read_u32(&join_suffix(base, "_min")).map_err(|(e, _)| e)?;
        let max_speed = read_u32(&join_suffix(base, "_max")).map_err(|(e, _)| e)?;

        let input_file = File::open(join_suffix(base, "_input")).map_err(Error::FanOpen)?;

        eprintln!(
            "  Found {}: min={}, max={} RPM",
            fan_path.name, min_speed, max_speed
        );

        Ok(Self {
            name: fan_path.name,
            base_path: fan_path.base_path,
            manual_file: None,
            output_file: None,
            input_file,
            config,
            min_speed,
            max_speed,
        })
    }

    pub fn open_control_files(&mut self) -> Result<()> {
        let mut open_opts = OpenOptions::new();
        open_opts.write(true).truncate(true);

        self.manual_file = Some(
            open_opts
                .open(join_suffix(&self.base_path, "_manual"))
                .map_err(Error::FanOpen)?,
        );
        self.output_file = Some(
            open_opts
                .open(join_suffix(&self.base_path, "_output"))
                .map_err(Error::FanOpen)?,
        );
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn min_speed(&self) -> u32 {
        self.min_speed
    }

    pub fn max_speed(&self) -> u32 {
        self.max_speed
    }

    pub fn sensor_indices(&self) -> Option<&[usize]> {
        self.config.sensor_indices.as_deref()
    }

    pub fn sensor_aggregation(&self) -> SensorAggregation {
        self.config.sensor_aggregation
    }

    pub fn ramp_down_rate(&self) -> f32 {
        self.config.ramp_down_rate
    }

    pub fn set_manual(&self, enabled: bool) -> Result<()> {
        let file = self
            .manual_file
            .as_ref()
            .ok_or(Error::FanWrite(std::io::Error::new(
                std::io::ErrorKind::Other,
                "control files not opened",
            )))?;
        (&*file)
            .write_all(if enabled { b"1" } else { b"0" })
            .map_err(Error::FanWrite)
    }

    pub fn set_speed(&self, mut speed: u32) -> Result<()> {
        let file = self
            .output_file
            .as_ref()
            .ok_or(Error::FanWrite(std::io::Error::new(
                std::io::ErrorKind::Other,
                "control files not opened",
            )))?;
        if speed < self.min_speed {
            speed = self.min_speed;
        } else if speed >= self.max_speed {
            speed = self.max_speed + 260;
        }

        write!(&*file, "{speed}").map_err(Error::FanWrite)
    }

    pub fn set_speed_percent(&self, percent: u8) -> Result<()> {
        let range = self.max_speed - self.min_speed;
        let speed = self.min_speed + range * u32::from(percent) / 100;
        self.set_speed(speed)
    }

    pub fn calc_speed(&self, temp: u8) -> u32 {
        if temp <= self.config.low_temp {
            return self.min_speed;
        }
        if temp >= self.config.high_temp {
            return self.max_speed;
        }

        let temp = u32::from(temp);
        let low_temp = u32::from(self.config.low_temp);
        let high_temp = u32::from(self.config.high_temp);
        let range = (self.max_speed - self.min_speed) as f32;
        let temp_range = (high_temp - low_temp) as f32;
        let t = (temp - low_temp) as f32;

        let fraction = match self.config.speed_curve {
            SpeedCurve::Linear => t / temp_range,
            SpeedCurve::Exponential => t.powi(3) / temp_range.powi(3),
            SpeedCurve::Logarithmic => t.log(temp_range),
        };

        (fraction * range) as u32 + self.min_speed
    }

    pub fn read_rpm(&mut self) -> Result<u32> {
        let mut buf = String::new();
        self.input_file
            .read_to_string(&mut buf)
            .map_err(Error::MinSpeedRead)?;
        self.input_file.rewind().map_err(Error::MinSpeedRead)?;
        let rpm = buf.trim().parse::<u32>().map_err(Error::MinSpeedParse)?;
        Ok(rpm)
    }
}

fn join_suffix(base: &PathBuf, suffix: &str) -> PathBuf {
    let file_name = base.file_name().unwrap().to_str().unwrap();
    base.with_file_name(format!("{file_name}{suffix}"))
}

fn read_u32(path: &PathBuf) -> std::result::Result<u32, (Error, ())> {
    let content = fs::read_to_string(path).map_err(|e| (Error::MinSpeedRead(e), ()))?;
    content
        .trim()
        .parse::<u32>()
        .map_err(|e| (Error::MinSpeedParse(e), ()))
}
