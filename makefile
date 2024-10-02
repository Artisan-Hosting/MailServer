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
	@cat << EOF > $(BINARY_NAME).service
[Unit]
Description=Mailing Server Application
After=network.target

[Service]
Type=simple
ExecStart=$(INSTALL_DIR)/$(BINARY_NAME)
WorkingDirectory=$(CONFIG_DIR)
User=ais
Group=ais
Restart=on-failure
RestartSec=10
StandardOutput=append:$(LOG_DIR)/$(BINARY_NAME).log
StandardError=append:$(LOG_DIR)/$(BINARY_NAME).err

[Install]
WantedBy=multi-user.target
EOF

	install -m 644 $(BINARY_NAME).service $(SERVICE_FILE)
	rm -f $(BINARY_NAME).service
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
