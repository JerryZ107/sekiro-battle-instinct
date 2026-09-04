use std::path::Path;

/// `# 启动信息print窗口: 关` / `# boot console: off` (legacy: `启动窗口`)
pub fn parse_boot_console_comment(line: &str) -> Option<bool> {
    let text = line.trim();
    if !text.starts_with('#') {
        return None;
    }
    let body = text.trim_start_matches('#').trim();
    let key = body.split([':', '：']).next()?.trim().to_ascii_lowercase();
    if key != "启动信息print窗口" && key != "启动窗口" && key != "boot console" {
        return None;
    }
    let val = body.split([':', '：']).nth(1)?.trim().to_ascii_lowercase();
    Some(parse_on_off_token(&val))
}

fn parse_on_off_token(val: &str) -> bool {
    match val {
        "开" | "on" | "1" | "true" | "yes" | "是" => true,
        "关" | "off" | "0" | "false" | "no" | "否" => false,
        other => {
            log::warn!("Unknown boot-console value `{other}`; defaulting to off");
            false
        }
    }
}

/// Read boot print window flag from cfg; default **off** when omitted.
pub fn boot_console_from_file(path: impl AsRef<Path>) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    for line in content.lines() {
        if let Some(v) = parse_boot_console_comment(line) {
            return v;
        }
    }
    false
}
