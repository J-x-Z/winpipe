# Winpipe

Windows 端的 Wayland 协议处理器（实验性项目）

## 当前状态

⚠️ **这是一个实验性项目：**

- ✅ 实现了基础的 Wayland 协议解析
- ✅ 可以接受 WSL Wayland 应用的连接
- ✅ 支持 wl_display, wl_registry, wl_compositor, xdg_shell 等协议
- ❌ **无法传递共享内存**（wl_shm 需要 Unix 文件描述符，TCP 不支持）

## 说明

本项目**不是 waypipe 的移植**，只是参考了 waypipe 的概念，代码完全从零编写。

## 安装

```powershell
git clone https://github.com/J-x-Z/winpipe.git
cd winpipe
cargo build --release
```

## 使用方法

```powershell
cargo run --release --bin winpipe server --port 9998
```

### WSL 端

```bash
WIN_IP=$(ip route | grep default | cut -d' ' -f3)
socat UNIX-LISTEN:/tmp/wayland-wp,fork TCP:$WIN_IP:9998 &
export WAYLAND_DISPLAY=/tmp/wayland-wp
foot  # 可以连接，但因 shm 限制无法显示
```

## 已知限制

- **无法传递 fd**：Wayland 的 wl_shm 使用 Unix 文件描述符传递共享内存，TCP 无法实现这一功能
- **无输入事件**：键盘鼠标事件未实现

## 系统要求

- Windows 10+
- Rust 1.70+
- WSL2 with socat

## 许可证

MIT

（本项目未使用任何 waypipe 代码，仅参考其概念）
