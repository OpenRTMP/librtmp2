//! Logging subsystem
//!
//! Mirrors `src/core/log.h` and `src/core/log.c`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

/// Log levels.
///
/// Ordered from most to least verbose; the library emits a message when its
/// level is greater than or equal to the level set by [`set_level`]. The
/// discriminants are the raw values accepted by the C ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub enum LogLevel {
    /// Verbose diagnostics used while debugging the protocol layer.
    Debug = 0,
    /// Informational messages about connection lifecycle.
    Info = 1,
    /// Recoverable problems that do not stop the connection.
    Warn = 2,
    /// Errors that cause a connection or operation to fail.
    Error = 3,
}

/// Log callback function type.
pub type LogFn = fn(level: LogLevel, msg: &str, userdata: *mut u8);

struct LogCallback {
    cb: Option<LogFn>,
    /// Stored as usize so the struct is Send+Sync; cast back to *mut u8 on use.
    /// The caller is responsible for keeping the pointed-to data alive.
    userdata: usize,
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);
static LOG_CALLBACK: Mutex<LogCallback> = Mutex::new(LogCallback {
    cb: None,
    userdata: 0,
});

const LEVEL_STRINGS: [&str; 4] = ["DEBUG", "INFO", "WARN", "ERROR"];

/// Set the minimum log level.
///
/// Messages below this level are discarded before any formatting work happens.
/// The setting is process-global and applies to every connection.
///
/// # Examples
///
/// ```
/// use librtmp2::log::{LogLevel, set_level};
///
/// set_level(LogLevel::Debug);
/// set_level(LogLevel::Info);
/// ```
pub fn set_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Set a custom log callback.
///
/// The callback receives every message at or above the current
/// [`LogLevel`], replacing the default `stderr` output.
///
/// `userdata` is passed back unchanged. The library stores it as a `usize` and
/// never dereferences it, but the pointed-to data must stay alive for as long as
/// the callback is installed.
///
/// The callback is invoked outside the internal lock, so it may itself log
/// without deadlocking.
///
/// # Examples
///
/// ```
/// use librtmp2::log::{LogLevel, set_callback};
///
/// fn sink(level: LogLevel, msg: &str, _userdata: *mut u8) {
///     let _ = (level, msg); // forward to your own logging framework
/// }
///
/// set_callback(sink, std::ptr::null_mut());
/// ```
pub fn set_callback(cb: LogFn, userdata: *mut u8) {
    let mut guard = LOG_CALLBACK.lock().unwrap_or_else(|e| e.into_inner());
    guard.cb = Some(cb);
    guard.userdata = userdata as usize;
}

/// Log a message at the given level.
pub fn log(level: LogLevel, file: &str, line: u32, args: std::fmt::Arguments<'_>) {
    if level < LOG_LEVEL.load(Ordering::Relaxed).into() {
        return;
    }

    let msg = format!("{}", args);
    let basename = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let level_str = LEVEL_STRINGS[level as usize];
    let full_msg = format!("[{}] {}:{}: {}", level_str, basename, line, msg);

    // Snapshot callback under the lock, then call outside the lock so that a
    // callback that itself logs does not deadlock on LOG_CALLBACK.
    let snapshot = {
        let guard = LOG_CALLBACK.lock().unwrap_or_else(|e| e.into_inner());
        guard.cb.map(|cb| (cb, guard.userdata))
    };

    if let Some((cb, userdata)) = snapshot {
        cb(level, &full_msg, userdata as *mut u8);
    } else {
        eprintln!("{}", full_msg);
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
