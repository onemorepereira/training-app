#!/bin/sh
# Privileged Linux setup for training-app.
#
# Compiled into the binary (include_str!) and run in ONE authentication prompt as:
#   pkexec /bin/sh -c "<this file>" training-app-setup <step>...
# The script text is a constant; the only inputs are the step tokens below
# (positional args, validated before anything runs) and, for `udev`, the rule
# file content on stdin. No paths are passed in and no temp files are used.
#
# Steps:  udev | bluez=apt-get | bluez=dnf | bluez=pacman | bluetooth
# Exit codes: 2 bad usage, 10-12 udev, 20 bluez, 30 bluetooth
# (126/127 are reserved by pkexec for dismissed/denied authentication).
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH

if [ "$#" -eq 0 ]; then
    echo "training-app-setup: no steps given" >&2
    exit 2
fi

for step in "$@"; do
    case "$step" in
        udev | bluetooth | bluez=apt-get | bluez=dnf | bluez=pacman) ;;
        *)
            echo "training-app-setup: unknown step: $step" >&2
            exit 2
            ;;
    esac
done

for step in "$@"; do
    echo "training-app-setup: $step" >&2
    case "$step" in
        udev)
            install -m 0644 -o root -g root /dev/stdin /etc/udev/rules.d/99-ant-usb.rules || exit 10
            udevadm control --reload-rules || exit 11
            udevadm trigger || exit 12
            ;;
        bluez=apt-get)
            DEBIAN_FRONTEND=noninteractive apt-get install -y bluez || exit 20
            ;;
        bluez=dnf)
            dnf install -y bluez || exit 20
            ;;
        bluez=pacman)
            pacman -S --noconfirm bluez bluez-utils || exit 20
            ;;
        bluetooth)
            systemctl enable --now bluetooth || exit 30
            ;;
    esac
done
