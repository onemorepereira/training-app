use serde::Serialize;
use std::process::Command;

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

fn check_udev_rules() -> bool {
    match std::fs::read_to_string("/etc/udev/rules.d/99-ant-usb.rules") {
        Ok(contents) => {
            contents.contains("0fcf") && contents.contains("1008") && contents.contains("1009")
        }
        Err(_) => false,
    }
}

fn check_bluez_installed() -> bool {
    Command::new("bluetoothctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

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

fn is_pkexec_available() -> bool {
    Command::new("which")
        .arg("pkexec")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn check() -> PrereqStatus {
    let udev_rules = check_udev_rules();
    let bluez_installed = check_bluez_installed();
    let bluetooth_service = check_bluetooth_service();
    let pkexec_available = is_pkexec_available();
    PrereqStatus {
        udev_rules,
        bluez_installed,
        bluetooth_service,
        all_met: udev_rules && bluez_installed && bluetooth_service,
        pkexec_available,
    }
}

fn detect_package_manager() -> Option<&'static str> {
    for cmd in ["apt-get", "dnf", "pacman"] {
        if Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(cmd);
        }
    }
    None
}

/// Build the list of commands to run as root via pkexec.
/// Each entry is a Vec of arguments to pass to `pkexec` directly,
/// avoiding shell script construction and shell injection risks.
fn build_fix_commands(status: &PrereqStatus, udev_rules_source: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();

    if !status.udev_rules {
        commands.push(vec![
            "cp".into(),
            udev_rules_source.into(),
            "/etc/udev/rules.d/99-ant-usb.rules".into(),
        ]);
        commands.push(vec![
            "udevadm".into(),
            "control".into(),
            "--reload-rules".into(),
        ]);
        commands.push(vec!["udevadm".into(), "trigger".into()]);
    }

    if !status.bluez_installed {
        if let Some(pm) = detect_package_manager() {
            let install_args: Vec<String> = match pm {
                "apt-get" => vec!["apt-get", "install", "-y", "bluez"],
                "dnf" => vec!["dnf", "install", "-y", "bluez"],
                "pacman" => vec!["pacman", "-S", "--noconfirm", "bluez", "bluez-utils"],
                _ => unreachable!(),
            }
            .into_iter()
            .map(String::from)
            .collect();
            commands.push(install_args);
        }
    }

    if !status.bluetooth_service {
        commands.push(vec![
            "systemctl".into(),
            "enable".into(),
            "--now".into(),
            "bluetooth".into(),
        ]);
    }

    commands
}

pub fn fix(udev_rules_source: &str) -> FixResult {
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

    let commands = build_fix_commands(&status, udev_rules_source);

    for args in &commands {
        let output = Command::new("pkexec").args(args).output();
        match output {
            Ok(o) if !o.status.success() => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let new_status = check();
                return FixResult {
                    success: false,
                    message: format!("Fix failed at '{}': {}", args.join(" "), stderr.trim()),
                    status: new_status,
                };
            }
            Err(e) => {
                let new_status = check();
                return FixResult {
                    success: false,
                    message: format!("Failed to run pkexec {}: {}", args.join(" "), e),
                    status: new_status,
                };
            }
            Ok(_) => {} // success, continue to next command
        }
    }

    let new_status = check();
    FixResult {
        success: new_status.all_met,
        message: if new_status.all_met {
            "All prerequisites fixed successfully.".into()
        } else {
            "Fix completed but some prerequisites still unmet.".into()
        },
        status: new_status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_commands_all_missing() {
        let status = PrereqStatus {
            udev_rules: false,
            bluez_installed: false,
            bluetooth_service: false,
            all_met: false,
            pkexec_available: true,
        };
        let cmds = build_fix_commands(&status, "/tmp/99-ant-usb.rules");
        // udev: cp, udevadm control, udevadm trigger (3 commands)
        // bluez: conditional on package manager
        // bluetooth: systemctl (1 command)
        assert!(cmds.len() >= 4, "expected at least 4 commands, got {}", cmds.len());

        assert_eq!(cmds[0], vec!["cp", "/tmp/99-ant-usb.rules", "/etc/udev/rules.d/99-ant-usb.rules"]);
        assert_eq!(cmds[1], vec!["udevadm", "control", "--reload-rules"]);
        assert_eq!(cmds[2], vec!["udevadm", "trigger"]);

        // Last command is always the systemctl enable
        let last = cmds.last().unwrap();
        assert_eq!(last, &vec!["systemctl", "enable", "--now", "bluetooth"]);
    }

    #[test]
    fn fix_commands_only_udev_missing() {
        let status = PrereqStatus {
            udev_rules: false,
            bluez_installed: true,
            bluetooth_service: true,
            all_met: false,
            pkexec_available: true,
        };
        let cmds = build_fix_commands(&status, "/opt/rules/99-ant-usb.rules");
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0], vec!["cp", "/opt/rules/99-ant-usb.rules", "/etc/udev/rules.d/99-ant-usb.rules"]);
        assert_eq!(cmds[1], vec!["udevadm", "control", "--reload-rules"]);
        assert_eq!(cmds[2], vec!["udevadm", "trigger"]);
    }

    #[test]
    fn fix_commands_all_met_produces_no_commands() {
        let status = PrereqStatus {
            udev_rules: true,
            bluez_installed: true,
            bluetooth_service: true,
            all_met: true,
            pkexec_available: true,
        };
        let cmds = build_fix_commands(&status, "/tmp/rules");
        assert!(cmds.is_empty());
    }

    #[test]
    fn fix_commands_bluez_missing_has_install_cmd() {
        let status = PrereqStatus {
            udev_rules: true,
            bluez_installed: false,
            bluetooth_service: true,
            all_met: false,
            pkexec_available: true,
        };
        let cmds = build_fix_commands(&status, "/tmp/rules");
        // No udev or systemctl commands
        for cmd in &cmds {
            assert_ne!(cmd[0], "udevadm");
            assert_ne!(cmd[0], "systemctl");
        }
        // If a package manager is available, there should be an install command
        if detect_package_manager().is_some() {
            assert_eq!(cmds.len(), 1);
            let install = &cmds[0];
            assert!(
                install[0] == "apt-get" || install[0] == "dnf" || install[0] == "pacman",
                "expected a package manager command, got {:?}",
                install
            );
        }
    }

    #[test]
    fn fix_commands_path_with_special_chars_is_passed_verbatim() {
        // The whole point of this fix: paths with shell-special characters
        // are passed as discrete arguments, not interpolated into a script.
        let status = PrereqStatus {
            udev_rules: false,
            bluez_installed: true,
            bluetooth_service: true,
            all_met: false,
            pkexec_available: true,
        };
        let evil_path = "/tmp/it's a \"test\" && rm -rf /";
        let cmds = build_fix_commands(&status, evil_path);
        // The path must appear as a single, unmodified argument
        assert_eq!(cmds[0][1], evil_path);
    }
}
