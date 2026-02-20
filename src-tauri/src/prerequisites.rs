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

fn build_fix_script(status: &PrereqStatus, udev_rules_source: &str) -> String {
    let mut script = String::from("set -e\n");

    if !status.udev_rules {
        script.push_str(&format!(
            "cp '{}' /etc/udev/rules.d/99-ant-usb.rules\n\
             udevadm control --reload-rules\n\
             udevadm trigger\n",
            udev_rules_source
        ));
    }

    if !status.bluez_installed {
        if let Some(pm) = detect_package_manager() {
            let install_cmd = match pm {
                "apt-get" => "apt-get install -y bluez",
                "dnf" => "dnf install -y bluez",
                "pacman" => "pacman -S --noconfirm bluez bluez-utils",
                _ => unreachable!(),
            };
            script.push_str(install_cmd);
            script.push('\n');
        }
    }

    if !status.bluetooth_service {
        script.push_str("systemctl enable --now bluetooth\n");
    }

    script
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

    let script = build_fix_script(&status, udev_rules_source);

    let output = Command::new("pkexec")
        .args(["/bin/bash", "-c", &script])
        .output();

    let new_status = check();

    match output {
        Ok(o) if o.status.success() => FixResult {
            success: new_status.all_met,
            message: if new_status.all_met {
                "All prerequisites fixed successfully.".into()
            } else {
                "Fix completed but some prerequisites still unmet.".into()
            },
            status: new_status,
        },
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            FixResult {
                success: false,
                message: format!("Fix failed: {}", stderr.trim()),
                status: new_status,
            }
        }
        Err(e) => FixResult {
            success: false,
            message: format!("Failed to run pkexec: {}", e),
            status: new_status,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_script_all_missing() {
        let status = PrereqStatus {
            udev_rules: false,
            bluez_installed: false,
            bluetooth_service: false,
            all_met: false,
            pkexec_available: true,
        };
        let script = build_fix_script(&status, "/tmp/99-ant-usb.rules");
        assert!(script.contains("set -e"));
        assert!(script.contains("cp '/tmp/99-ant-usb.rules' /etc/udev/rules.d/99-ant-usb.rules"));
        assert!(script.contains("udevadm control --reload-rules"));
        assert!(script.contains("udevadm trigger"));
        // BlueZ install command depends on detected package manager, but service line is always present
        assert!(script.contains("systemctl enable --now bluetooth"));
    }

    #[test]
    fn fix_script_only_udev_missing() {
        let status = PrereqStatus {
            udev_rules: false,
            bluez_installed: true,
            bluetooth_service: true,
            all_met: false,
            pkexec_available: true,
        };
        let script = build_fix_script(&status, "/opt/rules/99-ant-usb.rules");
        assert!(script.contains("cp '/opt/rules/99-ant-usb.rules'"));
        assert!(script.contains("udevadm"));
        assert!(!script.contains("systemctl"));
        // Should not contain any install command
        assert!(!script.contains("apt-get"));
        assert!(!script.contains("dnf"));
        assert!(!script.contains("pacman"));
    }

    #[test]
    fn fix_script_all_met() {
        let status = PrereqStatus {
            udev_rules: true,
            bluez_installed: true,
            bluetooth_service: true,
            all_met: true,
            pkexec_available: true,
        };
        let script = build_fix_script(&status, "/tmp/rules");
        assert_eq!(script.trim(), "set -e");
    }

    #[test]
    fn fix_script_bluez_missing_has_install_cmd() {
        // This test verifies that when BlueZ is missing and a package manager is detected,
        // the script contains an install command. Since detect_package_manager() is live,
        // we just verify the script structure is correct.
        let status = PrereqStatus {
            udev_rules: true,
            bluez_installed: false,
            bluetooth_service: true,
            all_met: false,
            pkexec_available: true,
        };
        let script = build_fix_script(&status, "/tmp/rules");
        // Should not contain udev or systemctl commands
        assert!(!script.contains("udevadm"));
        assert!(!script.contains("systemctl"));
        // If a package manager is available, there should be an install command
        if detect_package_manager().is_some() {
            assert!(
                script.contains("apt-get install")
                    || script.contains("dnf install")
                    || script.contains("pacman -S")
            );
        }
    }
}
