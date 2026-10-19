use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PrereqStatus {
    pub udev_rules: bool,
    pub bluez_installed: bool,
    pub bluetooth_service: bool,
    pub all_met: bool,
    pub pkexec_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FixResult {
    pub success: bool,
    pub message: String,
    pub status: PrereqStatus,
}

// macOS and Windows handle BLE natively (Core Bluetooth / WinRT) and don't
// need udev rules, BlueZ, or systemd. Return "all met" on non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub fn check() -> PrereqStatus {
    PrereqStatus {
        udev_rules: true,
        bluez_installed: true,
        bluetooth_service: true,
        all_met: true,
        pkexec_available: false,
    }
}

#[cfg(not(target_os = "linux"))]
pub fn fix() -> FixResult {
    FixResult {
        success: true,
        message: "No prerequisites needed on this platform.".into(),
        status: check(),
    }
}

#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};

/// Privileged setup script, run verbatim via `pkexec /bin/sh -c`. A compile-time
/// constant: nothing is ever interpolated into it (design ADR-4).
#[cfg(target_os = "linux")]
pub const SETUP_SCRIPT: &str = include_str!("../resources/linux-setup.sh");

/// The udev rule, embedded so the fix never depends on the bundle's resource
/// dir (an AppImage's FUSE mount is unreadable by root) or a temp file.
#[cfg(target_os = "linux")]
pub const UDEV_RULE: &str = include_str!("../resources/99-ant-usb.rules");

#[cfg(target_os = "linux")]
const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/99-ant-usb.rules";

/// `$0` for the setup script; shows up in its error messages.
#[cfg(target_os = "linux")]
const SCRIPT_NAME: &str = "training-app-setup";

#[cfg(target_os = "linux")]
fn rule_is_current(contents: &str) -> bool {
    contents.contains("0fcf") && contents.contains("1008") && contents.contains("1009")
}

#[cfg(target_os = "linux")]
fn check_udev_rules() -> bool {
    std::fs::read_to_string(UDEV_RULE_PATH)
        .map(|c| rule_is_current(&c))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn check_bluez_installed() -> bool {
    Command::new("bluetoothctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn check_bluetooth_service() -> bool {
    Command::new("systemctl")
        .args(["is-active", "bluetooth"])
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout).trim() == "active"
        })
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn command_exists(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
pub fn check() -> PrereqStatus {
    let udev_rules = check_udev_rules();
    let bluez_installed = check_bluez_installed();
    let bluetooth_service = check_bluetooth_service();
    let pkexec_available = command_exists("pkexec");
    PrereqStatus {
        udev_rules,
        bluez_installed,
        bluetooth_service,
        all_met: udev_rules && bluez_installed && bluetooth_service,
        pkexec_available,
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    AptGet,
    Dnf,
    Pacman,
}

#[cfg(target_os = "linux")]
impl PackageManager {
    fn step_token(self) -> &'static str {
        match self {
            PackageManager::AptGet => "bluez=apt-get",
            PackageManager::Dnf => "bluez=dnf",
            PackageManager::Pacman => "bluez=pacman",
        }
    }
}

#[cfg(target_os = "linux")]
fn detect_package_manager() -> Option<PackageManager> {
    [
        ("apt-get", PackageManager::AptGet),
        ("dnf", PackageManager::Dnf),
        ("pacman", PackageManager::Pacman),
    ]
    .into_iter()
    .find(|(cmd, _)| command_exists(cmd))
    .map(|(_, pm)| pm)
}

/// One privileged process: program + argv + optional stdin content.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq)]
pub struct FixInvocation {
    pub program: &'static str,
    pub args: Vec<String>,
    pub stdin: Option<&'static str>,
}

/// Build the single `pkexec` invocation that fixes everything missing, or None
/// if there is nothing it can do. All variable input is allowlisted step tokens.
#[cfg(target_os = "linux")]
pub fn build_fix_invocation(
    status: &PrereqStatus,
    package_manager: Option<PackageManager>,
) -> Option<FixInvocation> {
    let mut steps: Vec<&'static str> = Vec::new();
    if !status.udev_rules {
        steps.push("udev");
    }
    if !status.bluez_installed {
        if let Some(pm) = package_manager {
            steps.push(pm.step_token());
        }
    }
    if !status.bluetooth_service {
        steps.push("bluetooth");
    }
    if steps.is_empty() {
        return None;
    }
    let mut args: Vec<String> = vec![
        "/bin/sh".into(),
        "-c".into(),
        SETUP_SCRIPT.into(),
        SCRIPT_NAME.into(),
    ];
    args.extend(steps.iter().map(|s| s.to_string()));
    Some(FixInvocation {
        program: "pkexec",
        stdin: (!status.udev_rules).then_some(UDEV_RULE),
        args,
    })
}

/// Run an invocation; returns (exit code, stderr). A `BrokenPipe` while writing
/// stdin is expected when pkexec exits before reading it (authentication
/// dismissed/denied), so it is not an error — the exit status decides.
#[cfg(target_os = "linux")]
fn run_invocation(inv: &FixInvocation) -> std::io::Result<(Option<i32>, String)> {
    use std::io::Write;
    let mut child = Command::new(inv.program)
        .args(&inv.args)
        .stdin(if inv.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let (Some(content), Some(mut pipe)) = (inv.stdin, child.stdin.take()) {
        match pipe.write_all(content.as_bytes()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        }
        // pipe dropped here → EOF for `install /dev/stdin`
    }
    let output = child.wait_with_output()?;
    Ok((
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    ))
}

/// Human-readable message for a failed setup run. Codes 126/127 come from
/// pkexec(1): "dismissed the authentication dialog" / "not authorized".
#[cfg(target_os = "linux")]
fn describe_failure(code: Option<i32>, stderr: &str) -> String {
    let what = match code {
        Some(126) => "Authentication was cancelled.".to_string(),
        Some(127) => "Authorization was denied or failed.".to_string(),
        Some(2) => "Internal error: invalid setup step.".to_string(),
        Some(10) => "Failed to install the udev rule.".to_string(),
        Some(11) | Some(12) => "Failed to reload udev rules.".to_string(),
        Some(20) => "Failed to install BlueZ.".to_string(),
        Some(30) => "Failed to enable the bluetooth service.".to_string(),
        Some(c) => format!("Setup failed (exit code {}).", c),
        None => "Setup was terminated by a signal.".to_string(),
    };
    if stderr.is_empty() || matches!(code, Some(126) | Some(127)) {
        what
    } else {
        format!("{} {}", what, stderr)
    }
}

#[cfg(target_os = "linux")]
pub fn fix() -> FixResult {
    let status = check();
    if status.all_met {
        return FixResult {
            success: true,
            message: "All prerequisites already met.".into(),
            status,
        };
    }

    if !status.pkexec_available {
        return FixResult {
            success: false,
            message: "pkexec is not available. Install polkit or run the fixes manually:\n\
                      - Copy udev rules: sudo cp <rules-file> /etc/udev/rules.d/99-ant-usb.rules && sudo udevadm control --reload-rules && sudo udevadm trigger\n\
                      - Install BlueZ: sudo <package-manager> install bluez\n\
                      - Enable bluetooth: sudo systemctl enable --now bluetooth"
                .into(),
            status,
        };
    }

    let Some(invocation) = build_fix_invocation(&status, detect_package_manager()) else {
        return FixResult {
            success: false,
            message: "No supported package manager (apt-get, dnf, pacman) found; install BlueZ manually.".into(),
            status,
        };
    };

    let message = match run_invocation(&invocation) {
        Ok((Some(0), _)) => None,
        Ok((code, stderr)) => Some(describe_failure(code, &stderr)),
        Err(e) => Some(format!("Failed to run pkexec: {}", e)),
    };

    let new_status = check();
    match message {
        Some(message) => FixResult {
            success: false,
            message,
            status: new_status,
        },
        None => FixResult {
            success: new_status.all_met,
            message: if new_status.all_met {
                "All prerequisites fixed successfully.".into()
            } else {
                "Fix completed but some prerequisites still unmet.".into()
            },
            status: new_status,
        },
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::*;

    fn status(udev: bool, bluez: bool, bt: bool) -> PrereqStatus {
        PrereqStatus {
            udev_rules: udev,
            bluez_installed: bluez,
            bluetooth_service: bt,
            all_met: udev && bluez && bt,
            pkexec_available: true,
        }
    }

    /// Run the embedded setup script unprivileged (never reaches a privileged
    /// command because validation fails first).
    fn run_script(steps: &[&str]) -> (Option<i32>, String) {
        let mut args: Vec<String> = vec!["-c".into(), SETUP_SCRIPT.into(), SCRIPT_NAME.into()];
        args.extend(steps.iter().map(|s| s.to_string()));
        let out = Command::new("/bin/sh")
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.code(), String::from_utf8_lossy(&out.stderr).to_string())
    }

    #[test]
    fn fix_invocation_all_missing_is_single_pkexec_call() {
        let inv = build_fix_invocation(&status(false, false, false), Some(PackageManager::Dnf))
            .unwrap();
        assert_eq!(inv.program, "pkexec");
        assert_eq!(
            inv.args,
            vec![
                "/bin/sh".to_string(),
                "-c".into(),
                SETUP_SCRIPT.into(),
                "training-app-setup".into(),
                "udev".into(),
                "bluez=dnf".into(),
                "bluetooth".into(),
            ]
        );
        assert_eq!(inv.stdin, Some(UDEV_RULE));
    }

    #[test]
    fn fix_invocation_without_udev_step_has_no_stdin() {
        let inv = build_fix_invocation(&status(true, false, true), Some(PackageManager::AptGet))
            .unwrap();
        assert_eq!(&inv.args[4..], &["bluez=apt-get".to_string()]);
        assert_eq!(inv.stdin, None);
    }

    #[test]
    fn fix_invocation_passes_no_paths_or_temp_files() {
        let inv = build_fix_invocation(&status(false, false, false), Some(PackageManager::Pacman))
            .unwrap();
        // args[2] is the constant script; everything after $0 is a plain token
        for step in &inv.args[4..] {
            assert!(!step.contains('/'), "step token looks like a path: {step}");
        }
        assert!(inv.args.iter().all(|a| !a.contains("/tmp/")));
    }

    #[test]
    fn fix_invocation_all_met_is_none() {
        assert_eq!(build_fix_invocation(&status(true, true, true), Some(PackageManager::Dnf)), None);
    }

    #[test]
    fn fix_invocation_only_bluez_missing_without_package_manager_is_none() {
        assert_eq!(build_fix_invocation(&status(true, false, true), None), None);
    }

    #[test]
    fn embedded_udev_rule_satisfies_the_check() {
        assert!(rule_is_current(UDEV_RULE));
    }

    #[test]
    fn setup_script_rejects_unknown_step_before_any_change() {
        let (code, stderr) = run_script(&["udev", "bogus"]);
        assert_eq!(code, Some(2));
        assert!(stderr.contains("unknown step: bogus"), "{stderr}");
        // `udev` was not attempted: validation precedes execution
        assert!(!stderr.contains("training-app-setup: udev"), "{stderr}");
    }

    #[test]
    fn setup_script_treats_shell_metacharacters_as_data() {
        let (code, stderr) = run_script(&["bluetooth; echo PWNED"]);
        assert_eq!(code, Some(2));
        assert!(!stderr.lines().any(|l| l.trim() == "PWNED"), "{stderr}");
        assert!(stderr.contains("unknown step: bluetooth; echo PWNED"), "{stderr}");
    }

    #[test]
    fn setup_script_requires_a_step() {
        assert_eq!(run_script(&[]).0, Some(2));
    }

    #[test]
    fn setup_script_has_non_interactive_package_installs() {
        assert!(SETUP_SCRIPT.contains("DEBIAN_FRONTEND=noninteractive apt-get install -y bluez"));
        assert!(SETUP_SCRIPT.contains("dnf install -y bluez"));
        assert!(SETUP_SCRIPT.contains("pacman -S --noconfirm bluez bluez-utils"));
    }

    #[test]
    fn closed_stdin_is_judged_by_exit_status() {
        // Child exits without reading stdin, like pkexec after a dismissed dialog
        let inv = FixInvocation {
            program: "/bin/sh",
            args: vec!["-c".into(), "exit 126".into()],
            stdin: Some(Box::leak("x".repeat(1 << 20).into_boxed_str())),
        };
        let (code, _) = run_invocation(&inv).unwrap();
        assert_eq!(code, Some(126));
        assert_eq!(describe_failure(code, ""), "Authentication was cancelled.");
    }

    #[test]
    fn describe_failure_maps_pkexec_denied_and_step_codes() {
        assert_eq!(describe_failure(Some(127), "noise"), "Authorization was denied or failed.");
        assert_eq!(
            describe_failure(Some(20), "training-app-setup: bluez=dnf"),
            "Failed to install BlueZ. training-app-setup: bluez=dnf"
        );
    }
}
