# Variables
BINARY_NAME=mailing_server
INSTALL_DIR=/usr/local/bin
CONFIG_DIR=/etc/$(BINARY_NAME)
SERVICE_FILE=/etc/systemd/system/$(BINARY_NAME).service
LOG_DIR=/var/log/$(BINARY_NAME)
TARGET_DIR=target/release

# Targets
.PHONY: all build install config service logs run clean

# Default target: Build, install, configure, and set up logs
all: build install config service logs

# Build the binary in release mode
build:
	cargo build --release

# Install the binary to the system
install: build
	install -d $(INSTALL_DIR)
	install $(TARGET_DIR)/$(BINARY_NAME) $(INSTALL_DIR)/$(BINARY_NAME)

# Create the configuration directory and copy the configuration files
config:
	install -d $(CONFIG_DIR)
	install Config.toml Overrides.toml $(CONFIG_DIR)

# Create the log directory
logs:
	install -d $(LOG_DIR)

# Install the systemd service file and reload daemon
service:
	@echo "Creating systemd service file for $(BINARY_NAME)"
	@{
		echo "[Unit]"
		echo "Description=Mailing Server Application"
		echo "After=network.target"
		echo ""
		echo "[Service]"
		echo "Type=simple"
		echo "ExecStart=$(INSTALL_DIR)/$(BINARY_NAME)"
		echo "WorkingDirectory=$(CONFIG_DIR)"
		echo "User=ais"
		echo "Group=ais"
		echo "Restart=on-failure"
		echo "RestartSec=10"
		echo "StandardOutput=append:$(LOG_DIR)/$(BINARY_NAME).log"
		echo "StandardError=append:$(LOG_DIR)/$(BINARY_NAME).err"
		echo ""
		echo "[Install]"
		echo "WantedBy=multi-user.target"
	} > $(SERVICE_FILE)

	install -m 644 $(SERVICE_FILE) $(SERVICE_FILE)
	systemctl daemon-reload && systemctl enable $(BINARY_NAME).service

# Run the binary with the correct working directory and capture logs
run:
	systemctl start $(BINARY_NAME).service

# Clean the build artifacts, configuration, and logs
clean:
	cargo clean
	rm -rf $(LOG_DIR) $(CONFIG_DIR) $(SERVICE_FILE)
	systemctl disable $(BINARY_NAME).service || true
	systemctl daemon-reload
