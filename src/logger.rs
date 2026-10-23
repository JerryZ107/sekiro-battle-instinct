use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::Local;
use log::{self, Level, LevelFilter};

// Temporary: Info so art-fire diagnostics show in release builds.
const LEVEL: LevelFilter = LevelFilter::Info;
/// Keep the active log bounded and keep at most one previous log.
/// Total disk use for battle_instinct.log* is therefore ~2x this value.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

struct RotatingFileInner {
    path: PathBuf,
    rotated_path: PathBuf,
    file: Option<File>,
    size: u64,
}

impl RotatingFileInner {
    fn rotate(&mut self) -> io::Result<()> {
        // Close the current handle before renaming on Windows.
        self.file = None;
        let _ = fs::remove_file(&self.rotated_path);
        if let Err(err) = fs::rename(&self.path, &self.rotated_path) {
            // Keep logging on the original file if rotation fails.
            self.file = Some(OpenOptions::new().create(true).append(true).open(&self.path)?);
            self.size = self.file.as_ref().unwrap().metadata()?.len();
            return Err(err);
        }

        self.file = Some(OpenOptions::new().create(true).append(true).open(&self.path)?);
        self.size = 0;
        Ok(())
    }
}

/// Size-capped writer used by `fern`. The old startup-only 5 MB check could not
/// stop a single long session from growing the log without bound.
struct RotatingFile {
    inner: Mutex<RotatingFileInner>,
}

impl RotatingFile {
    fn open(path: &Path) -> io::Result<Self> {
        let rotated_path = path.with_extension("log.1");
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let mut size = file.metadata()?.len();

        // Reclaim a log left oversized by an older build. Runtime rotation handles
        // the normal case below.
        if size >= MAX_LOG_BYTES {
            drop(file);
            let _ = fs::remove_file(path);
            file = OpenOptions::new().create(true).append(true).open(path)?;
            size = file.metadata()?.len();
        }

        Ok(Self {
            inner: Mutex::new(RotatingFileInner {
                path: path.to_path_buf(),
                rotated_path,
                file: Some(file),
                size,
            }),
        })
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log lock poisoned"))?;

        if inner.size > 0 && inner.size.saturating_add(buf.len() as u64) > MAX_LOG_BYTES {
            inner.rotate()?;
        }

        let file = inner
            .file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "log file is closed"))?;
        file.write_all(buf)?;
        inner.size = inner.size.saturating_add(buf.len() as u64);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log lock poisoned"))?;
        inner
            .file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "log file is closed"))?
            .flush()
    }
}

pub fn init(path: &Path) {
    let _: anyhow::Result<()> = (|| {
        let path = path.join("battle_instinct.log");
        let writer: Box<dyn Write + Send> = Box::new(RotatingFile::open(&path)?);
        fern::Dispatch::new()
            .format(|out, args, record| {
                out.finish(format_args!(
                    "{} [{:<3}] {}",
                    Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                    record.level().abbr(),
                    args
                ))
            })
            .level(LEVEL)
            .chain(writer)
            .apply()?;
        std::panic::set_hook(Box::new(|info| log::error!("{info}")));
        Ok(())
    })();
}

trait LevelExt {
    fn abbr(self) -> &'static str;
}

impl LevelExt for Level {
    fn abbr(self) -> &'static str {
        match self {
            Level::Error => "ERR",
            Level::Warn => "WRN",
            Level::Info => "INF",
            Level::Debug => "DBG",
            Level::Trace => "TRC",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_log_stays_bounded() {
        let dir = std::env::temp_dir().join(format!(
            "battle-instinct-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("battle_instinct.log");

        {
            let mut writer = RotatingFile::open(&path).unwrap();
            let chunk = vec![b'x'; 1024 * 1024];
            for _ in 0..6 {
                writer.write_all(&chunk).unwrap();
            }
            writer.flush().unwrap();
        }

        assert!(fs::metadata(&path).unwrap().len() <= MAX_LOG_BYTES);
        assert!(path.with_extension("log.1").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
