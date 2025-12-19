# Winpipe

Windows-native Wayland protocol handler (Experimental)

## Current Status

⚠️ **This is an experimental project:**

- ✅ Implements basic Wayland protocol parsing
- ✅ Can accept connections from WSL Wayland apps
- ✅ Supports wl_display, wl_registry, wl_compositor, xdg_shell protocols
- ❌ **Cannot pass shared memory** (wl_shm requires Unix file descriptors, not supported over TCP)

## Note

This project is **not a port of waypipe**. It only references waypipe's concepts. All code is written from scratch.

## Installation

```powershell
git clone https://github.com/J-x-Z/winpipe.git
cd winpipe
cargo build --release
```

## Usage

```powershell
cargo run --release --bin winpipe server --port 9998
```

### WSL Side

```bash
WIN_IP=$(ip route | grep default | cut -d' ' -f3)
socat UNIX-LISTEN:/tmp/wayland-wp,fork TCP:$WIN_IP:9998 &
export WAYLAND_DISPLAY=/tmp/wayland-wp
foot  # Can connect, but cannot display due to shm limitation
```

## Known Limitations

- **Cannot pass fd**: Wayland's wl_shm uses Unix file descriptors for shared memory, which cannot be passed over TCP
- **No input events**: Keyboard/mouse events not implemented

## Requirements

- Windows 10+
- Rust 1.70+
- WSL2 with socat

## License

MIT

(This project does not use any waypipe code, only references its concepts)
