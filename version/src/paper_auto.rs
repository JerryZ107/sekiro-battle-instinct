use std::{
    fs,
    io::Write,
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const EMBEDDED_PATCHER: &str = include_str!("../../tools/patch_paper_params.ps1");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn run(game_dir: &Path) {
    let cfg = game_dir.join("battle_instinct.cfg");
    let param = game_dir.join("mods/param/gameparam/gameparam.parambnd.dcx");
    if !cfg.is_file() || !param.is_file() {
        log_status(game_dir, "paper auto: cfg or param file missing; skipped");
        return;
    }

    // The patcher is a PowerShell + Add-Type startup (~1 s on this machine).
    // It used to run on every game launch even when neither the cfg nor the
    // param package had changed. A content fingerprint lets unchanged launches
    // skip that work while still reapplying immediately after either file or
    // the embedded patcher changes.
    let stamp_path = game_dir.join("battle_instinct_paper_auto.stamp");
    let Some(fingerprint) = patch_fingerprint(&cfg, &param) else {
        log_status(game_dir, "paper auto: cannot hash cfg/param; running patcher");
        return run_patcher(game_dir, &cfg, &param, &stamp_path, None);
    };
    let stamp = fs::read_to_string(&stamp_path).ok();
    if stamp.as_deref().map(str::trim) == Some(fingerprint.as_str()) {
        log_status(game_dir, "paper auto: cfg/param unchanged; skipped");
        return;
    }

    run_patcher(game_dir, &cfg, &param, &stamp_path, Some(fingerprint));
}

fn run_patcher(game_dir: &Path, cfg: &Path, param: &Path, stamp_path: &Path, fingerprint: Option<String>) {
    let temp_script = std::env::temp_dir().join(format!(
        "battle_instinct_params_{}_{}.ps1",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    if let Err(error) = fs::write(&temp_script, EMBEDDED_PATCHER) {
        log_status(game_dir, &format!("paper auto: write temp script failed: {error}"));
        return;
    }

    let result = Command::new("powershell.exe")
        .current_dir(game_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&temp_script)
        .args(["-Apply", "-Auto", "-Lang", "zh", "-ParamFile"])
        .arg(&param)
        .arg("-CfgFile")
        .arg(&cfg)
        .status();

    let _ = fs::remove_file(&temp_script);
    match result {
        Ok(status) if status.success() => {
            if let Some(fingerprint) = fingerprint {
                let _ = fs::write(stamp_path, fingerprint);
            }
            log_status(game_dir, "paper auto: param patch completed");
        }
        Ok(status) => {
            log_status(game_dir, &format!("paper auto: patcher exited with {status}"));
        }
        Err(error) => {
            log_status(game_dir, &format!("paper auto: failed to start powershell: {error}"));
        }
    }
}

fn patch_fingerprint(cfg: &Path, param: &Path) -> Option<String> {
    let cfg = fs::read(cfg).ok()?;
    let param = fs::read(param).ok()?;
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for chunk in [
        b"paper-auto-v1".as_slice(),
        cfg.as_slice(),
        param.as_slice(),
        EMBEDDED_PATCHER.as_bytes(),
    ] {
        hash = fnv1a(hash, &(chunk.len() as u64).to_le_bytes());
        hash = fnv1a(hash, chunk);
    }
    Some(format!("{hash:016x}\n"))
}

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn log_status(game_dir: &Path, message: &str) {
    let path = game_dir.join("battle_instinct_paper_auto.log");
    let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "[{unix}] {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_tracks_file_content() {
        let dir = std::env::temp_dir().join(format!(
            "paper-auto-fingerprint-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("battle_instinct.cfg");
        let param = dir.join("gameparam.parambnd.dcx");
        fs::write(&cfg, "paper cap fix: on\n").unwrap();
        fs::write(&param, b"param-v1").unwrap();

        let first = patch_fingerprint(&cfg, &param).unwrap();
        assert_eq!(patch_fingerprint(&cfg, &param).unwrap(), first);

        fs::write(&param, b"param-v2").unwrap();
        assert_ne!(patch_fingerprint(&cfg, &param).unwrap(), first);

        let _ = fs::remove_dir_all(&dir);
    }
}
