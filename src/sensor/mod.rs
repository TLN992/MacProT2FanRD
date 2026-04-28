pub mod discovery;
pub mod registry;

use crate::error::{Error, Result};
use std::fs::File;
use std::io::{Read, Seek};

pub enum SensorStatus {
    Active(u8),
    Unavailable,
    Error(String),
}

pub struct HwmonSensor {
    name: String,
    driver: String,
    temp_file: File,
}

impl HwmonSensor {
    pub fn new(name: String, driver: String, temp_file: File) -> Self {
        Self {
            name,
            driver,
            temp_file,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn driver(&self) -> &str {
        &self.driver
    }

    pub fn read_temp(&mut self) -> SensorStatus {
        let mut buf = String::new();
        match self.read_temp_inner(&mut buf) {
            Ok(t) => SensorStatus::Active(t),
            Err(Error::TempRead(_) | Error::TempSeek(_)) => SensorStatus::Unavailable,
            Err(e) => SensorStatus::Error(e.to_string()),
        }
    }

    fn read_temp_inner(&mut self, buf: &mut String) -> Result<u8> {
        self.temp_file
            .read_to_string(buf)
            .map_err(Error::TempRead)?;

        self.temp_file.rewind().map_err(Error::TempSeek)?;

        let millidegrees = buf.trim_end().parse::<i32>().map_err(Error::TempParse)?;
        buf.clear();
        Ok((millidegrees / 1000).clamp(0, 255) as u8)
    }
}
