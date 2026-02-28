# eithne kernel — top-level Makefile
#
# Targets:
#   make          — build kernel + user programs + ESP image
#   make kernel   — build only the kernel EFI binary
#   make user     — build only user-mode programs
#   make esp      — assemble the ESP directory tree
#   make run      — build everything and launch QEMU
#   make clippy   — run clippy with warnings-as-errors
#   make clean    — remove all build artifacts
#   make check    — cargo build + clippy

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

ARCH          ?= x86_64

# Rust / Cargo
CARGO         := cargo
CARGO_FLAGS   := --release

# QEMU exit: Ctrl-B X
QEMU_ECHR     := 0x02

ifeq ($(ARCH),x86_64)
  UEFI_TARGET := x86_64-unknown-uefi
  OVMF        := /usr/share/edk2/x64/OVMF.4m.fd
  QEMU        := qemu-system-x86_64
  BOOT_FILE   := BOOTX64.EFI
  QEMU_EXTRA  :=
  AS_FLAGS    := --64
else ifeq ($(ARCH),aarch64)
  UEFI_TARGET := aarch64-unknown-uefi
  OVMF        := /usr/share/edk2/aarch64/QEMU_EFI.fd
  QEMU        := qemu-system-aarch64
  BOOT_FILE   := BOOTAA64.EFI
  QEMU_EXTRA  := -machine virt -cpu cortex-a72
  AS_FLAGS    :=
else
  $(error Unsupported ARCH=$(ARCH). Use x86_64 or aarch64)
endif

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

BUILD_DIR     := target
ESP_DIR       := $(BUILD_DIR)/esp
EFI_BINARY    := $(BUILD_DIR)/$(UEFI_TARGET)/release/eithne.efi

# User programs
USER_SRC_DIR  := user
USER_LINK     := $(USER_SRC_DIR)/link.ld
USER_INIT_SRC := $(USER_SRC_DIR)/shell.S
USER_INIT_OBJ := $(BUILD_DIR)/user/shell.o
USER_INIT_ELF := $(BUILD_DIR)/user/init

# ESP layout
ESP_BOOT_DIR  := $(ESP_DIR)/EFI/BOOT
ESP_APP_DIR   := $(ESP_DIR)/EFI/eithne

# ---------------------------------------------------------------------------
# Phony targets
# ---------------------------------------------------------------------------

.PHONY: all kernel user esp run run-gui clippy check clean

all: esp

kernel: $(EFI_BINARY)

user: $(USER_INIT_ELF)

esp: $(ESP_BOOT_DIR)/$(BOOT_FILE) $(ESP_APP_DIR)/init

run: esp
	@echo "==> Launching QEMU ($(ARCH)) — serial only..."
	@echo "    Press Ctrl-B X to exit QEMU"
	@echo ""
	$(QEMU) \
		-bios $(OVMF) \
		-drive format=raw,file=fat:rw:$(ESP_DIR) \
		-net none \
		-nographic \
		-echr $(QEMU_ECHR) \
		$(QEMU_EXTRA)

run-gui: esp
	@echo "==> Launching QEMU ($(ARCH)) — GUI + serial..."
	@echo "    QEMU window: framebuffer output"
	@echo "    Terminal:     serial I/O"
	@echo "    Press Ctrl-B X to exit QEMU"
	@echo ""
	$(QEMU) \
		-bios $(OVMF) \
		-drive format=raw,file=fat:rw:$(ESP_DIR) \
		-net none \
		-serial stdio \
		-vga std \
		-echr $(QEMU_ECHR) \
		$(QEMU_EXTRA)

clippy:
	$(CARGO) clippy --target $(UEFI_TARGET) $(CARGO_FLAGS) -- -D warnings

check: kernel clippy

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR)/user $(ESP_DIR)

# ---------------------------------------------------------------------------
# Build rules
# ---------------------------------------------------------------------------

# Kernel EFI binary
$(EFI_BINARY): FORCE
	@echo "==> Building kernel for $(UEFI_TARGET)..."
	$(CARGO) build --target $(UEFI_TARGET) $(CARGO_FLAGS)

# User program: assemble
$(USER_INIT_OBJ): $(USER_INIT_SRC) | $(BUILD_DIR)/user
	@echo "==> Assembling $(USER_INIT_SRC)..."
	as $(AS_FLAGS) -o $@ $<

# User program: link
$(USER_INIT_ELF): $(USER_INIT_OBJ) $(USER_LINK) | $(BUILD_DIR)/user
	@echo "==> Linking user/init..."
	ld -T $(USER_LINK) -nostdlib -o $@ $(USER_INIT_OBJ)

# ESP: copy kernel
$(ESP_BOOT_DIR)/$(BOOT_FILE): $(EFI_BINARY) | $(ESP_BOOT_DIR)
	cp $< $@

# ESP: copy user init
$(ESP_APP_DIR)/init: $(USER_INIT_ELF) | $(ESP_APP_DIR)
	cp $< $@

# Directory creation
$(BUILD_DIR)/user $(ESP_BOOT_DIR) $(ESP_APP_DIR):
	mkdir -p $@

# Force cargo to always re-evaluate (it has its own dep tracking)
FORCE:
