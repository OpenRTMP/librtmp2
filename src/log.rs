//! Logging subsystem
//!
//! Mirrors `src/core/log.h` and `src/core/log.c`.

use std::sync::atomic::{AtomicU8, Ordering};

/// Log levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

/// Log callback function type.
pub type LogFn = fn(level: LogLevel, msg: &str, userdata: *mut u8);

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);
static mut LOG_FN: Option<LogFn> = None;
static mut LOG_USERDATA: *mut u8 = std::ptr::null_mut();

const LEVEL_STRINGS: [&str; 4] = ["DEBUG", "INFO", "WARN", "ERROR"];

/// Set the minimum log level.
pub fn set_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Set a custom log callback.
pub fn set_callback(cb: LogFn, userdata: *mut u8) {
    unsafe {
        LOG_FN = Some(cb);
        LOG_USERDATA = userdata;
    }
}

/// Log a message at the given level.
pub fn log(level: LogLevel, file: &str, line: u32, args: std::fmt::Arguments<'_>) {
    if level < LOG_LEVEL.load(Ordering::Relaxed).into() {
        return;
    }

    let msg = format!("{}", args);
    let basename = file.rsplit('/').next().unwrap_or(file);
    let level_str = LEVEL_STRINGS[level as usize];
    let full_msg = format!("[{}] {}:{}: {}", level_str, basename, line, msg);

    unsafe {
        if let Some(cb) = LOG_FN {
            cb(level, &full_msg, LOG_USERDATA);
        } else {
            eprintln!("{}", full_msg);
        }
    }
}

/// Log at debug level.
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Debug, file!(), line!(), format_args!($($arg)*))
    };
}

/// Log at info level.
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Info, file!(), line!(), format_args!($($arg)*))
    };
}

/// Log at warn level.
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Warn, file!(), line!(), format_args!($($arg)*))
    };
}

/// Log at error level.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::log::log($crate::log::LogLevel::Error, file!(), line!(), format_args!($($arg)*))
    };
}

// Allow LogLevel to be created from u8
impl From<u8> for LogLevel {
    fn from(v: u8) -> Self {
        match v {
            0 => LogLevel::Debug,
            1 => LogLevel::Info,
            2 => LogLevel::Warn,
            _ => LogLevel::Error,
        }
    }
}
