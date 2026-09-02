use std::{
    fs::{self},
    os::windows::fs::MetadataExt,
    path::Path,
};

use chrono::Local;
use log::{self, Level, LevelFilter};

// Temporary: Info so art-fire diagnostics show in release builds.
const LEVEL: LevelFilter = LevelFilter::Info;

pub fn init(path: &Path) {
    let _: anyhow::Result<()> = (|| {
        let path = path.join("battle_instinct.log");
        if let Ok(meta) = fs::metadata(&path)
            && meta.file_size() >= 5 * 1024 * 1024
        {
            let _ = fs::remove_file(&path);
        }
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
            .chain(fern::log_file(path)?)
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
