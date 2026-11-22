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
