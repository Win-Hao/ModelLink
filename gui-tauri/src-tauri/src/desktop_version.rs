//! Claude Desktop 版本探测与「键 → 最低版本」门槛（§3.8）。
//!
//! ModelLink 写的键是逐版本加进 Claude Desktop 的。给低版本写它不认识的键，
//! 轻则被忽略，重则整份配置过不了 schema 校验 —— 后者会让用户的网关直接失效。
//! 所以按检测到的版本决定写哪些键，并在设置页把「因版本过低不可用的能力」摊开告诉用户。

/// 键 → 该键起始支持的版本（取自 app.asar 里每个字段的 `availableInVersion`）。
///
/// ⚠️ 顺带记一个到期点：`inferenceGatewayAuthScheme` 的 `sso` / `auto` 两个取值
/// 2026-10-07 失效。ModelLink 固定写 `bearer`，不受影响 —— 别手滑改成那两个。
pub const KEY_MIN_VERSION: &[(&str, &str)] = &[
    ("chatTabEnabled", "1.13576.0"),
    ("labelOverride", "1.2581.0"),
    ("inferenceModelPricingEnabled", "1.37937.0"),
    ("inferenceModelPricing", "1.37937.0"),
    ("organizationInstructions", "1.37937.0"),
    ("inferenceStreamIdleTimeoutSec", "1.44121.1"),
    ("egressProxyUrl", "1.44121.1"),
    ("egressProxyPacUrl", "1.44121.1"),
];

/// 版本比较：把 `1.46388.3` 拆成数字元组比。段数不同时短的补 0。
/// 解析不了的段按 0 计 —— 宁可当成低版本少写一个键，也不要写坏配置。
pub fn version_at_least(have: &str, want: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0)).collect()
    };
    let (a, b) = (parse(have), parse(want));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        match x.cmp(&y) {
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Equal => {}
        }
    }
    true
}

/// 已检测到的 Claude Desktop 版本。`None` = 没探到（没装 / 装在非常规位置）。
#[derive(Clone, Debug, Default)]
pub struct VersionGate {
    pub version: Option<String>,
}

impl VersionGate {
    /// 探测本机 Claude Desktop 版本。
    pub fn detect() -> Self {
        Self { version: detect_version() }
    }

    /// 显式指定版本（单测用）。
    #[cfg(test)]
    pub fn with_version(v: Option<&str>) -> Self {
        Self { version: v.map(|s| s.to_string()) }
    }

    /// 这个键能不能写。
    ///
    /// 探不到版本时**全写** —— 探测失败远比「装了老版本」常见（装在非常规路径、
    /// 权限不足），因此保持 2.0 的行为，不因为读不到 plist 就砍掉用户的功能。
    pub fn allows(&self, key: &str) -> bool {
        let Some(have) = self.version.as_deref() else {
            return true;
        };
        match KEY_MIN_VERSION.iter().find(|(k, _)| *k == key) {
            Some((_, want)) => version_at_least(have, want),
            None => true, // 没登记门槛的键（2.0 就在写的那几个）一律写
        }
    }

    /// 该版本起才认识的键能不能写（不在 [`KEY_MIN_VERSION`] 表里、自带版本号的键用，如一键配置）。
    /// 探不到版本时同样全写，理由见 [`Self::allows`]。
    pub fn allows_since(&self, min_version: &str) -> bool {
        self.version.as_deref().is_none_or(|have| version_at_least(have, min_version))
    }

    /// 因版本过低而不可用的键，给设置页展示。
    pub fn unavailable_keys(&self) -> Vec<&'static str> {
        KEY_MIN_VERSION
            .iter()
            .filter(|(k, _)| !self.allows(k))
            .map(|(k, _)| *k)
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn detect_version() -> Option<String> {
    // 常规安装位置优先；用户也可能装在 ~/Applications
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        "/Applications/Claude.app/Contents/Info.plist".to_string(),
        format!("{home}/Applications/Claude.app/Contents/Info.plist"),
    ];
    for path in candidates {
        if !std::path::Path::new(&path).exists() {
            continue;
        }
        // Info.plist 常是二进制格式，交给系统工具读，别自己解析
        let out = std::process::Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", "Print CFBundleShortVersionString", &path])
            .output()
            .ok()?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_version() -> Option<String> {
    // Squirrel/Electron 约定：安装目录下是 app-<版本> 一层层堆着，取最高的那个
    let base = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join("AnthropicClaude");
    let mut best: Option<String> = None;
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(v) = name.strip_prefix("app-") else { continue };
        if best.as_deref().map(|b| version_at_least(v, b)).unwrap_or(true) {
            best = Some(v.to_string());
        }
    }
    best
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn detect_version() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_handles_real_build_numbers() {
        assert!(version_at_least("1.46388.3", "1.44121.1"));
        assert!(!version_at_least("1.44121.0", "1.44121.1"));
        assert!(version_at_least("1.44121.1", "1.44121.1"));
        // 段数不同：短的补 0
        assert!(version_at_least("2", "1.99999.9"));
        assert!(version_at_least("1.44121.1", "1.44121"));
        assert!(!version_at_least("1.44121", "1.44121.1"));
        // 按段比数值，不能按字符串（13576 > 2581）
        assert!(version_at_least("1.13576.0", "1.2581.0"));
    }

    #[test]
    fn unparseable_segments_degrade_to_zero_not_panic() {
        assert!(!version_at_least("1.beta.0", "1.1.0"));
        assert!(version_at_least("1.1.0", "1.beta.0"));
        assert!(!version_at_least("", "1.0.0"));
    }

    #[test]
    fn old_desktop_loses_only_the_keys_it_cannot_take() {
        let gate = VersionGate::with_version(Some("1.40000.0"));
        // 1.37937.0 起：费率 / 组织指令能写
        assert!(gate.allows("inferenceModelPricingEnabled"));
        assert!(gate.allows("organizationInstructions"));
        // 1.44121.1 起：心跳超时 / 网络代理不能写
        assert!(!gate.allows("inferenceStreamIdleTimeoutSec"));
        assert!(!gate.allows("egressProxyUrl"));
        // 2.0 就在写的那几个键没有门槛，永远写
        assert!(gate.allows("inferenceGatewayBaseUrl"));
        assert_eq!(
            gate.unavailable_keys(),
            vec!["inferenceStreamIdleTimeoutSec", "egressProxyUrl", "egressProxyPacUrl"]
        );
    }

    #[test]
    fn ancient_desktop_loses_chat_tab_too() {
        let gate = VersionGate::with_version(Some("1.10000.0"));
        assert!(!gate.allows("chatTabEnabled"));
        assert!(gate.allows("labelOverride"));
    }

    #[test]
    fn undetected_version_writes_everything() {
        // 探不到版本远比「装了老版本」常见（非常规路径、权限），
        // 不能因为读不到 plist 就砍掉用户的功能
        let gate = VersionGate::with_version(None);
        for (k, _) in KEY_MIN_VERSION {
            assert!(gate.allows(k), "{k}");
        }
        assert!(gate.unavailable_keys().is_empty());
    }

    #[test]
    fn current_desktop_takes_every_key() {
        let gate = VersionGate::with_version(Some("1.46388.3"));
        assert!(gate.unavailable_keys().is_empty());
    }
}
