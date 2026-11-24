#!/bin/bash
set -Eeuo pipefail

if [[ "$#" != 8 ]]; then
    echo "Usage: $0 QEMU BASE_QCOW2 BOOT_DIR WARDEN WARDEN_SHA256 LINUX_NODE_TAR LINUX_NODE_MODULES OUTPUT" >&2
    exit 2
fi

qemu=$1
image=$2
boot=$3
warden=$4
warden_sha=$5
node_archive=$6
modules=$7
result=$8
root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
for path in "$@"; do
    case "$path" in
        /*) ;;
        "$warden_sha") ;;
        *) echo "Use absolute paths for VM inputs and output." >&2; exit 2 ;;
    esac
done
test "$(uname -m)" = arm64
test -x "$qemu"
test -f "$modules/@cloudflare/workerd-linux-arm64/bin/workerd"
printf '%s  %s\n' "$warden_sha" "$warden" | shasum -a 256 -c
umask 077
mkdir "$result"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

mkdir -p "$scratch"/seed "$scratch"/payload/{bin,crates/veil-decoy-edge/build,lab/v2,migrations}
payload="$scratch/payload"
cp Cargo.lock "$payload/"
cp lab/v2/{fixture.js,trial.mjs,warden.mjs,package-lock.json} "$payload/lab/v2/"
cp crates/veil-decoy-edge/worker.js "$payload/crates/veil-decoy-edge/"
cp crates/veil-decoy-edge/build/veil_decoy_edge{.js,_bg.wasm} "$payload/crates/veil-decoy-edge/build/"
cp migrations/0001_decoy_events.sql "$payload/migrations/"
cp -R "$modules" "$payload/lab/v2/node_modules"
install -m 0755 "$warden" "$payload/bin/warden-observe"
printf '%s\n' "$warden_sha" >"$payload/warden.sha256"
tar -xOf "$node_archive" node-v26.13.1-linux-arm64/bin/node >"$payload/bin/node"
chmod +x "$payload/bin/node"
COPYFILE_DISABLE=1 tar --no-xattrs --no-acls -czf "$scratch/seed/payload.tgz" -C "$payload" .

cp lab/v2/vm-guest.sh "$scratch/seed/guest.sh"
printf 'instance-id: veil-decoy-v2\nlocal-hostname: veil-decoy-v2\n' >"$scratch/seed/meta-data"
printf 'version: 2\nethernets: {}\n' >"$scratch/seed/network-config"
cat >"$scratch/seed/user-data" <<'CLOUD_CONFIG'
#cloud-config
disable_root: true
ssh_pwauth: false
package_update: false
package_upgrade: false
users: []
runcmd:
  - [sh, -c, "mkdir -p /mnt/veil-decoy-seed && mount -t iso9660 -o ro /dev/vdb /mnt/veil-decoy-seed && /bin/bash /mnt/veil-decoy-seed/guest.sh"]
CLOUD_CONFIG

hdiutil makehybrid -quiet -iso -joliet -default-volume-name cidata \
    -o "$scratch/cidata.iso" "$scratch/seed"
qemu_dir=$(dirname "$qemu")
firmware="$qemu_dir/../share/qemu/edk2-aarch64-code.fd"
"$qemu_dir/qemu-img" create -q -f qcow2 -F qcow2 -b "$image" "$scratch/guest.qcow2"

shasum -a 256 "$qemu" "$image" "$boot/linux-arm64" "$boot/initrd" "$warden" \
    "$node_archive" "$scratch/seed/payload.tgz" >"$result/inputs.sha256"

node - "$qemu" "$firmware" "$scratch" "$boot" "$result" <<'NODE'
const { spawnSync } = require("node:child_process");
const [qemu, firmware, scratch, boot, out] = process.argv.slice(2);
const run = spawnSync(qemu, [
    "-machine", "virt,accel=hvf", "-cpu", "host", "-smp", "4", "-m", "3072",
    "-nodefaults", "-nic", "none", "-display", "none", "-monitor", "none",
    "-serial", "file:" + out + "/serial.log", "-bios", firmware, "-no-reboot",
    "-drive", "file=" + scratch + "/guest.qcow2,if=virtio,format=qcow2",
    "-drive", "file=" + scratch + "/cidata.iso,if=virtio,format=raw,readonly=on",
    "-kernel", boot + "/linux-arm64", "-initrd", boot + "/initrd", "-append",
    "root=/dev/vda1 ro console=ttyAMA0 systemd.mask=systemd-networkd-wait-online.service systemd.mask=serial-getty@ttyAMA0.service",
], { stdio: ["ignore", "inherit", "inherit"], timeout: 145_000, killSignal: "SIGKILL" });
if (run.error) throw run.error;
if (run.status !== 0) throw new Error("QEMU exited with status " + run.status);
NODE

node - "$result" <<'NODE'
const { readFileSync, writeFileSync } = require("node:fs");
const { join } = require("node:path");
const out = process.argv[2];
const serial = readFileSync(join(out, "serial.log"), "utf8");
if (!serial.includes("VEIL_DECOY_VM_OFFLINE") || !serial.includes("VEIL_DECOY_VM_COMPLETE")) {
    throw new Error("The isolated VM did not complete; see serial.log");
}
for (const [marker, file] of [["RECEIPT", "receipt.json"], ["WARDEN", "warden.ndjson"]]) {
    const encoded = new RegExp("VEIL_DECOY_" + marker + " ([A-Za-z0-9+/=]+)").exec(serial)?.[1];
    if (!encoded) throw new Error("Missing " + marker);
    writeFileSync(join(out, file), Buffer.from(encoded, "base64"), { flag: "wx" });
}
NODE
(
    cd "$result"
    shasum -a 256 receipt.json warden.ndjson >SHA256SUMS
    shasum -a 256 -c SHA256SUMS
)
