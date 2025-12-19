# Winpipe

Windows-native Waypipe implementation - Wayland protocol proxy for WSL applications.

## Features

- 🔌 **Wayland Compositor** - Implements core Wayland protocol
- 📦 **LZ4 Compression** - Efficient data transfer
- 🔄 **Mirror Buffers** - Delta sync for surface updates
- 🌐 **TCP Transport** - Cross-OS communication

## Architecture

```
WSL Wayland App → socat → TCP:9998 → winpipe (protocol) → win-way (display)
```

## Installation

```powershell
git clone https://github.com/J-x-Z/winpipe.git
cd winpipe
cargo build --release
```

## Usage

### Windows Side
```powershell
cargo run --release --bin winpipe server --port 9998
```

### WSL Side
```bash
# Install dependencies
sudo apt install socat

# Connect to winpipe
WIN_IP=$(ip route | grep default | cut -d' ' -f3)
rm -f /tmp/wayland-wp
socat UNIX-LISTEN:/tmp/wayland-wp,fork TCP:$WIN_IP:9998 &

# Run Wayland application
export WAYLAND_DISPLAY=/tmp/wayland-wp
foot  # or any Wayland app
```

## CLI Options

```
winpipe server [OPTIONS]
  -p, --port <PORT>    TCP port to listen on (default: 9999)
  -d, --debug          Enable debug logging
```

## Implemented Protocols

| Interface | Version | Status |
|-----------|---------|--------|
| wl_display | 1 | ✅ |
| wl_registry | 1 | ✅ |
| wl_compositor | 5 | ✅ |
| wl_shm | 1 | ⚠️ (no fd passing) |
| wl_output | 4 | ✅ |
| wl_seat | 8 | ⚠️ |
| xdg_wm_base | 5 | ✅ |
| xdg_surface | 5 | ✅ |
| xdg_toplevel | 5 | ✅ |

## Known Limitations

- **No fd passing over TCP** - wl_shm buffers require file descriptors which cannot be passed over TCP. Use waypipe on WSL side for full support.
- **No input events** - Keyboard/mouse events not yet implemented

## Requirements

- Windows 10+
- Rust 1.70+
- WSL2 with socat

## Related Projects

- [win-way](https://github.com/J-x-Z/win-way) - GPU renderer for displaying Wayland surfaces
- [waypipe](https://gitlab.freedesktop.org/mstoeckl/waypipe) - Original Wayland network proxy

## License

MIT
