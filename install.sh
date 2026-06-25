#!/bin/bash
set -e

# TaskFlow Native Linux Installer

echo "=============================================="
echo "          Installing TaskFlow                 "
echo "=============================================="

# 1. Build release binaries
echo "Building TaskFlow in release mode..."
cargo build --workspace --release

# 2. Setup user local bin directory
echo "Installing binaries to $HOME/.local/bin..."
mkdir -p "$HOME/.local/bin"
cp target/release/taskflow-gui "$HOME/.local/bin/"
cp target/release/taskflow-daemon "$HOME/.local/bin/"

# 3. Setup systemd user service
echo "Installing systemd user service..."
mkdir -p "$HOME/.config/systemd/user"
cp packaging/systemd/taskflow-daemon.service "$HOME/.config/systemd/user/"

echo "Reloading systemd user daemon and enabling taskflow-daemon..."
systemctl --user daemon-reload
systemctl --user enable --now taskflow-daemon

# 4. Install desktop icon and launcher
echo "Installing desktop launcher..."
mkdir -p "$HOME/.local/share/applications"
mkdir -p "$HOME/.local/share/icons"

# Copy the premium app icon
cp assets/taskflow_icon.jpg "$HOME/.local/share/icons/taskflow.jpg"

# Write the .desktop file
cat <<EOF > "$HOME/.local/share/applications/taskflow.desktop"
[Desktop Entry]
Name=TaskFlow
Comment=Fast, native task manager that syncs with Google Tasks
Exec=$HOME/.local/bin/taskflow-gui
Icon=$HOME/.local/share/icons/taskflow.jpg
Terminal=false
Type=Application
Categories=Office;Utility;
Keywords=tasks;todo;google;sync;
EOF

chmod +x "$HOME/.local/share/applications/taskflow.desktop"

# 5. Install OAuth credentials if present
echo "Installing OAuth credentials..."
mkdir -p "$HOME/.config/taskflow"
if ls client_secret_*.json 1> /dev/null 2>&1; then
    cp client_secret_*.json "$HOME/.config/taskflow/"
    echo "Copied client_secret_*.json to $HOME/.config/taskflow/"
elif [ -f "oauth_client.json" ]; then
    cp oauth_client.json "$HOME/.config/taskflow/"
    echo "Copied oauth_client.json to $HOME/.config/taskflow/"
else
    echo "Warning: No OAuth credentials found in current directory. You will need to place them in ~/.config/taskflow/ manually."
fi

echo "=============================================="
echo "TaskFlow has been installed successfully!"
echo "Launcher: $HOME/.local/share/applications/taskflow.desktop"
echo "Daemon: taskflow-daemon is active in background"
echo "=============================================="
