#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-x86_64}"
PROFILE="${2:-debug}"

case "$TARGET" in
    x86_64)
        UEFI_TARGET="x86_64-unknown-uefi"
        OVMF="/usr/share/edk2/x64/OVMF.4m.fd"
        QEMU="qemu-system-x86_64"
        BOOT_FILE="BOOTX64.EFI"
        QEMU_EXTRA=""
        ;;
    aarch64)
        UEFI_TARGET="aarch64-unknown-uefi"
        OVMF="/usr/share/edk2/aarch64/QEMU_EFI.fd"
        QEMU="qemu-system-aarch64"
        BOOT_FILE="BOOTAA64.EFI"
        QEMU_EXTRA="-machine virt -cpu cortex-a72"
        ;;
    *)
        echo "Usage: $0 [x86_64|aarch64] [debug|release]"
        exit 1
        ;;
esac

echo "==> Building for $UEFI_TARGET ($PROFILE)..."
if [ "$PROFILE" = "release" ]; then
    cargo build --target "$UEFI_TARGET" --release
else
    cargo build --target "$UEFI_TARGET"
fi

EFI_BINARY="target/$UEFI_TARGET/$PROFILE/oxeidia-kernel.efi"

if [ ! -f "$EFI_BINARY" ]; then
    echo "ERROR: $EFI_BINARY not found"
    exit 1
fi

# Create ESP (EFI System Partition) directory structure
ESP_DIR="target/esp"
mkdir -p "$ESP_DIR/EFI/BOOT"
cp "$EFI_BINARY" "$ESP_DIR/EFI/BOOT/$BOOT_FILE"

echo "==> Launching QEMU ($TARGET)..."
echo "    Press Ctrl-B X to exit QEMU"
echo ""

$QEMU \
    -bios "$OVMF" \
    -drive format=raw,file=fat:rw:"$ESP_DIR" \
    -net none \
    -nographic \
    -echr 0x02 \
    $QEMU_EXTRA
