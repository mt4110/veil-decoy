#!/bin/bash
set -Eeuo pipefail

exec >/dev/ttyAMA0 2>&1
phase=preflight
finish() {
    local rc=$?
    if ((rc)); then
        echo "VEIL_DECOY_VM_FAILURE $phase $rc"
    fi
    sync
    systemctl poweroff --no-block
}
trap finish EXIT

test "$(uname -m)" = aarch64
test "$(ls /sys/class/net)" = lo
test -z "$(ip -4 route show default)"
test -z "$(ip -6 route show default)"
test "$(stat -f -c %T /sys/fs/cgroup)" = cgroup2fs
ip link set lo up
echo 'VEIL_DECOY_VM_OFFLINE'

phase=unpack
run=$(mktemp -d /run/veil-decoy-v2.XXXXXX)
mount -t tmpfs -o mode=0700,size=768M,exec,nodev,nosuid tmpfs "$run"
tar -xzf /mnt/veil-decoy-seed/payload.tgz -C "$run"
cd "$run"
export PATH="$run/bin:$PATH"
export VEIL_TRIAL_OUTPUT="$run/evidence"
export VEIL_WARDEN_BINARY="$run/bin/warden-observe"
export VEIL_WARDEN_SHA256
VEIL_WARDEN_SHA256=$(cat warden.sha256)

phase=trial
timeout --kill-after=5s 80s node lab/v2/trial.mjs
printf 'VEIL_DECOY_RECEIPT '
base64 -w 0 evidence/receipt.json
printf '\nVEIL_DECOY_WARDEN '
base64 -w 0 evidence/warden.ndjson
printf '\nVEIL_DECOY_VM_COMPLETE\n'
