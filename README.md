# Raspberry Pi MJPEG Camera Streamer

A high-performance MJPEG camera streaming server for Raspberry Pi, built with Rust. 
Uses GStreamer for hardware-accelerated video capture and Axum for efficient HTTP streaming.

## Auther

David Ron  [send remarks to rd789x@gmail.com]

## Features

- **Low latency streaming**: Direct camera-to-browser MJPEG stream
- **Efficient memory usage**: Latest-frame semantics (no frame buffering)
- **Hardware acceleration**: Uses libcamera and hardware JPEG encoding
- **Optimized for tunnels**: Configurable quality/framerate for bandwidth-constrained environments
- **TCP optimizations**: Keepalive and Nagle disable for reliable streaming
- **Multiple concurrent clients**: Efficient frame sharing without duplication

## Requirements

### Hardware
- Raspberry Pi (3/4/5 or Zero 2 W recommended)
- Compatible camera module (Camera Module v2, v3, or HQ Camera)

### Software
- Rust (1.70 or later)
- GStreamer 1.0 with libcamera plugin
- libcamera

## Installation

### 1. Install System Dependencies

```bash
# Update system
sudo apt update && sudo apt upgrade -y

# Install GStreamer and libcamera
sudo apt install -y \
    gstreamer1.0-tools \
    gstreamer1.0-plugins-base \
    gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad \
    gstreamer1.0-libcamera \
    libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev \
    libcamera-dev

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### 2. Clone and Build

```bash
git clone https://github.com/rdx789/rpi-mjpeg-streamer.git
cd rpi-mjpeg-streamer
cargo build --release
```

## Usage

### Basic Usage

```bash
cargo run --release
```

The server will start on `http://0.0.0.0:8080`

- Web interface: `http://<raspberry-pi-ip>:8080/`
- Direct stream: `http://<raspberry-pi-ip>:8080/stream`

### Configuration

Edit the GStreamer pipeline string in `src/main.rs` to adjust settings:

```rust
let pipeline_str = "\
    libcamerasrc ! \
    video/x-raw,format=NV12,width=1280,height=960,framerate=25/1 ! \
    jpegenc quality=70 ! \
    appsink name=sink emit-signals=true max-buffers=1 drop=true";
```

**Common adjustments:**
- **Resolution**: Change `width` and `height` (e.g., `640x480`, `1920x1080`)
- **Framerate**: Adjust `framerate=25/1` (e.g., `30/1`, `15/1`)
- **Quality**: Modify `quality=70` (1-100, higher = better quality, larger size)

### Systemd Service (Optional)

Create `/etc/systemd/system/camera-stream.service`:

```ini
[Unit]
Description=MJPEG Camera Streamer
After=network.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/rpi-mjpeg-streamer
ExecStart=/home/pi/rpi-mjpeg-streamer/target/release/rpi-mjpeg-streamer
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
Alias=camera.service
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable camera-stream.service
sudo systemctl start camera-stream.service
```

## Performance Tuning

### For Remote Access / Tunnels

If streaming over limited bandwidth (e.g., via Cloudflare Tunnel, ngrok):

```rust
// Lower resolution and framerate
width=640,height=480,framerate=15/1
// Reduce quality
jpegenc quality=50
```

### For Local Network

```rust
// Higher quality settings
width=1920,height=1080,framerate=30/1
jpegenc quality=85
```

## Troubleshooting

### Camera Not Detected

```bash
# Check if camera is detected
rpicam-hello --list-cameras

# Test camera
rpicam-still -o test.jpg
```

### GStreamer Pipeline Issues

```bash
# Test the pipeline directly
gst-launch-1.0 libcamerasrc ! video/x-raw,width=1280,height=960 ! jpegenc ! fakesink
```

### Port Already in Use

Change the port in `src/main.rs`:
```rust
TcpListener::bind("0.0.0.0:8080")  // Change 8080 to another port
```

## Architecture

- **GStreamer Pipeline**: Captures from libcamera, encodes to JPEG
- **Watch Channel**: Latest-frame distribution (constant memory)
- **Axum Web Server**: Serves HTTP/MJPEG streams
- **TCP Optimizations**: Keepalive + no-delay for tunnel reliability

## License

MIT

Copyright (c) 2025 David Ron - rd789x@gmail.com


## Contributing

Contributions welcome! Please open an issue or pull request.

## Acknowledgments

- Built with [Axum](https://github.com/tokio-rs/axum)
- Uses [GStreamer](https://gstreamer.freedesktop.org/)
- Powered by [Rust](https://www.rust-lang.org/)
