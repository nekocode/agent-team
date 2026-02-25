// ============================================================
// update - 自更新
// ============================================================

use anyhow::{bail, Context, Result};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 版本比较: latest > current 返回 true
fn compare_versions(current: &str, latest: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };

    let cur = parse(current);
    let lat = parse(latest);

    for i in 0..cur.len().max(lat.len()) {
        let c = cur.get(i).copied().unwrap_or(0);
        let l = lat.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }
    false
}

/// 从 npm registry 查询最新版本
/// 有新版返回 Some(version)，否则 None
fn check_update(current: &str) -> Result<Option<String>> {
    let output = std::process::Command::new("npm")
        .args(["view", "agent-team", "version"])
        .output()
        .context("failed to run npm")?;

    if !output.status.success() {
        bail!("npm view failed");
    }

    let latest = String::from_utf8(output.stdout)
        .context("invalid npm output")?
        .trim()
        .to_string();

    if latest.is_empty() {
        bail!("empty version from npm");
    }

    if compare_versions(current, &latest) {
        Ok(Some(latest))
    } else {
        Ok(None)
    }
}

/// 执行自更新
pub fn run_update() -> Result<()> {
    println!("Checking for updates...");

    match check_update(VERSION)? {
        None => {
            println!("Already up to date ({})", VERSION);
        }
        Some(latest) => {
            println!("Updating agent-team: {} -> {}", VERSION, latest);

            let status = std::process::Command::new("npm")
                .args(["install", "-g", "agent-team@latest"])
                .status()
                .context("failed to run npm")?;

            if !status.success() {
                bail!("npm install failed");
            }

            println!("Updated successfully!");
        }
    }

    Ok(())
}

// ============================================================
// 单元测试
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_version() {
        assert!(!compare_versions("0.1.0", "0.1.0"));
    }

    #[test]
    fn older_version() {
        assert!(!compare_versions("0.4.5", "0.4.4"));
        assert!(!compare_versions("1.0.0", "0.9.9"));
    }

    #[test]
    fn newer_version() {
        assert!(compare_versions("0.1.0", "0.1.1"));
        assert!(compare_versions("0.1.0", "0.2.0"));
        assert!(compare_versions("0.1.0", "1.0.0"));
    }

    #[test]
    fn edge_cases() {
        // 缺失的版本号默认 0
        assert!(compare_versions("0.4", "0.4.1"));
        assert!(!compare_versions("0.4.1", "0.4"));
        // 进位
        assert!(compare_versions("0.9.9", "0.10.0"));
    }
}
