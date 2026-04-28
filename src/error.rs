#[allow(dead_code)]
#[derive(Debug)]
pub enum Error {
    NotRoot,
    NoFan,

    TempRead(std::io::Error),
    TempSeek(std::io::Error),
    TempParse(std::num::ParseIntError),

    MinSpeedRead(std::io::Error),
    MinSpeedParse(std::num::ParseIntError),
    MaxSpeedRead(std::io::Error),
    MaxSpeedParse(std::num::ParseIntError),

    FanOpen(std::io::Error),
    FanWrite(std::io::Error),

    PidRead(std::io::Error),
    PidWrite(std::io::Error),
    PidDelete(std::io::Error),
    AlreadyRunning,

    Signal(std::io::Error),

    Glob(glob::PatternError),

    ConfigRead(std::io::Error),
    ConfigCreate(std::io::Error),
    ConfigParse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRoot => write!(f, "Fan daemon must be run as root"),
            Self::NoFan => write!(f, "No fans found"),
            Self::TempRead(e) => write!(f, "Temperature sensor cannot be read: {e}"),
            Self::TempSeek(e) => write!(f, "Temperature sensor cannot be seeked: {e}"),
            Self::TempParse(e) => write!(f, "Temperature sensor cannot be parsed: {e}"),
            Self::MinSpeedRead(e) => write!(f, "Cannot read minimum fan speed: {e}"),
            Self::MinSpeedParse(e) => write!(f, "Cannot parse minimum fan speed: {e}"),
            Self::MaxSpeedRead(e) => write!(f, "Cannot read maximum fan speed: {e}"),
            Self::MaxSpeedParse(e) => write!(f, "Cannot parse maximum fan speed: {e}"),
            Self::FanOpen(e) => write!(f, "Cannot open fan controller handle: {e}"),
            Self::FanWrite(e) => write!(f, "Cannot write to fan controller: {e}"),
            Self::PidRead(e) => write!(f, "Cannot read pid file: {e}"),
            Self::PidWrite(e) => write!(f, "Cannot write pid file: {e}"),
            Self::PidDelete(e) => write!(f, "Cannot delete pid file: {e}"),
            Self::AlreadyRunning => write!(f, "Fan daemon is already running"),
            Self::Signal(e) => write!(f, "Cannot setup shutdown signals: {e}"),
            Self::Glob(e) => write!(f, "Invalid glob pattern: {e}"),
            Self::ConfigRead(e) => write!(f, "Cannot read config file: {e}"),
            Self::ConfigCreate(e) => write!(f, "Cannot create config file: {e}"),
            Self::ConfigParse(e) => write!(f, "Cannot parse config file: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<glob::PatternError> for Error {
    fn from(e: glob::PatternError) -> Self {
        Self::Glob(e)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
