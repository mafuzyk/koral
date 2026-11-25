#!/usr/bin/env python3
"""Measure terminal input-to-photon latency with stdlib-only / session-native tools.

Methodology (Wayland Hyprland or KDE/KWin, compositor-native):
  1. A generated Python helper runs inside the target terminal as its command.
     It puts the PTY in raw mode, renders a full-screen red sentinel, and
     writes `BENCH_READY` to `BENCH_LAT_READY`.
  2. We wait for READY, take a screenshot baseline (`org.kde.KWin.ScreenShot2`
     or `spectacle` on KDE/KWin, `grim` on Hyprland/wlroots), then inject a
     synthetic key (`org_kde_kwin_fake_input` or `ydotool` on KDE/KWin,
     Hyprland's Lua dispatcher `hl.dsp.send_key_state` on Hyprland).
     The helper reads the PTY directly and only acknowledges the exact expected
     sequence (F12 is `ESC [ 2 4 ~`) in `BENCH_LAT_ACK`, then renders a green
     sentinel. No external benchmark shell or FIFO is involved.
  3. A synchronous capture loop repeatedly captures the workspace and
     hashes the pixels/PNG bytes. A sample is valid only when the first differing
     frame and the PTY acknowledgement are both observed before the deadline.
     `time.perf_counter_ns()` sampled immediately before key injection gives
     the start timestamp; the first differing frame gives the end timestamp.
     A timeout or a wrong key produces status "unavailable", never a number.
  4. Percentiles use linear interpolation between nearest samples.

This measures the full path: key injection -> compositor -> terminal PTY ->
renderer -> compositor frame -> screen capture. The PTY acknowledgement proves
that the injected key reached the process inside the terminal, which is the
fact missing from screenshot-only latency figures. When prerequisites are
missing the tool exits with status "unavailable" and a reason instead of
inventing numbers.

X11 can use xdotool for injection and ImageMagick `import` for capture when present.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import pty
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
from typing import Any


KEY_ACK = "BENCH_KEY_OK"
KEY_TIMEOUT = "BENCH_KEY_TIMEOUT"
RESULT_KEYS = (
    "terminal",
    "binary",
    "version",
    "methodology",
    "samples_ms",
    "key_events",
    "p50_ms",
    "p95_ms",
    "p99_ms",
    "status",
    "reason",
)


# Generated helper. It runs as the terminal's command, reads the PTY in raw
# mode, and only reports success for the exact byte sequence injected by this
# harness. Paths and timing come from the environment so one helper can serve
# every run.
PTY_HELPER_SOURCE = '''#!/usr/bin/env python3
"""Generated PTY key confirmation helper for scripts/bench-latency.py."""

import json
import os
import select
import sys
import termios
import time
import tty


def write_marker(path, text):
    directory = os.path.dirname(path)
    if directory:
        os.makedirs(directory, exist_ok=True)
    temporary = path + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        handle.write(text)
    os.replace(temporary, path)


def draw(color):
    frame = "\\x1b[H\\x1b[2J" + color + (" " * 120 + "\\n") * 60
    os.write(1, frame.encode("ascii"))


def main():
    ready_path = os.environ["BENCH_LAT_READY"]
    ack_path = os.environ["BENCH_LAT_ACK"]
    expected = bytes.fromhex(os.environ["BENCH_LAT_KEY_HEX"])
    timeout = float(os.environ.get("BENCH_LAT_KEY_TIMEOUT", "5"))
    hold = float(os.environ.get("BENCH_LAT_HOLD", "3"))
    previous = None
    try:
        previous = termios.tcgetattr(0)
        tty.setraw(0, termios.TCSANOW)
    except (OSError, termios.error, ValueError):
        previous = None
    draw("\\x1b[48;2;255;0;0m")
    write_marker(ready_path, "BENCH_READY\\n")
    deadline = time.monotonic() + timeout
    buffer = b""
    status = "BENCH_KEY_TIMEOUT"
    detail = {}
    while time.monotonic() < deadline:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        readable, _, _ = select.select([0], [], [], min(0.1, remaining))
        if not readable:
            continue
        try:
            chunk = os.read(0, 4096)
        except OSError:
            break
        if not chunk:
            break
        buffer = (buffer + chunk)[-256:]
        index = buffer.find(expected)
        if index >= 0:
            status = "BENCH_KEY_OK"
            detail = {
                "key_hex": expected.hex(),
                "observed_hex": buffer[index:index + len(expected)].hex(),
                "received_ns": time.perf_counter_ns(),
                "wall_ns": time.time_ns(),
            }
            break
    write_marker(ack_path, status + "\\n" + json.dumps(detail) + "\\n")
    if status == "BENCH_KEY_OK":
        draw("\\x1b[48;2;0;255;0m")
        time.sleep(hold)
    else:
        time.sleep(min(hold, 0.5))
    if previous is not None:
        try:
            termios.tcsetattr(0, termios.TCSANOW, previous)
        except (OSError, termios.error, ValueError):
            pass
    return 0 if status == "BENCH_KEY_OK" else 2


if __name__ == "__main__":
    sys.exit(main())
'''


def percentile(samples: list[float], q: float) -> float | None:
    """Linear-interpolation percentile; q in [0, 1]."""
    if not samples:
        return None
    values = sorted(samples)
    if len(values) == 1:
        return round(values[0], 3)
    pos = (len(values) - 1) * q
    lo = math.floor(pos)
    hi = math.ceil(pos)
    if lo == hi:
        return round(values[lo], 3)
    value = values[lo] + (values[hi] - values[lo]) * (pos - lo)
    return round(value, 3)


def run(cmd: list[str], *, timeout: float = 10.0, stdin: bytes | None = None) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(cmd, input=stdin, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)


def shutil_which(name: str) -> str | None:
    return shutil.which(name)


def terminal_version(binary: str) -> str:
    try:
        result = run([binary, "--version"], timeout=5)
        text = (result.stdout + result.stderr).decode("utf-8", "replace").strip()
        if text:
            return text.splitlines()[0]
    except (OSError, subprocess.TimeoutExpired):
        pass
    return "unknown"


def expected_key_sequence(key: str) -> bytes:
    """Only keys with an unambiguous, well-known PTY byte sequence."""
    sequences = {"F12": b"\x1b[24~"}
    if key not in sequences:
        raise RuntimeError(f"unsupported benchmark key: {key}")
    return sequences[key]


def expected_evdev_keycode(key: str) -> int:
    """Linux evdev keycode corresponding to a supported benchmark key."""
    keycodes = {"F12": 88}
    if key not in keycodes:
        raise RuntimeError(f"unsupported benchmark key: {key}")
    return keycodes[key]


_KWIN_SCREENSHOT2_IFACE: Any = None


def _kwin_screenshot2_iface() -> Any:
    global _KWIN_SCREENSHOT2_IFACE
    if _KWIN_SCREENSHOT2_IFACE is None:
        import dbus

        bus = dbus.SessionBus()
        obj = bus.get_object("org.kde.KWin", "/org/kde/KWin/ScreenShot2")
        _KWIN_SCREENSHOT2_IFACE = dbus.Interface(obj, "org.kde.KWin.ScreenShot2")
    return _KWIN_SCREENSHOT2_IFACE


def _capture_kwin_screenshot2() -> bytes:
    import dbus

    iface = _kwin_screenshot2_iface()
    read_fd, write_fd = os.pipe()
    try:
        try:
            iface.CaptureWorkspace(
                dbus.Dictionary({}, signature="sv"),
                dbus.types.UnixFd(write_fd),
            )
        finally:
            os.close(write_fd)
        chunks: list[bytes] = []
        while True:
            chunk = os.read(read_fd, 65_536)
            if not chunk:
                break
            chunks.append(chunk)
        return b"".join(chunks)
    finally:
        os.close(read_fd)


def _wayland_socket_path() -> str | None:
    display = os.environ.get("WAYLAND_DISPLAY", "")
    if not display:
        return None
    if os.path.isabs(display):
        path = display
    else:
        runtime_dir = os.environ.get("XDG_RUNTIME_DIR", "")
        if not runtime_dir:
            return None
        path = os.path.join(runtime_dir, display)
    return path if os.path.exists(path) else None


def _wl_send(sock: socket.socket, obj_id: int, opcode: int, payload: bytes = b"") -> None:
    size = 8 + len(payload)
    sock.sendall(struct.pack("=II", obj_id, (size << 16) | opcode) + payload)


def _wl_str(text: str) -> bytes:
    raw = text.encode("utf-8") + b"\x00"
    pad = (4 - (len(raw) % 4)) % 4
    return struct.pack("=I", len(raw)) + raw + (b"\x00" * pad)


def _wl_connect_and_find_global(
    sock_path: str, target_iface: str, timeout: float = 2.0
) -> tuple[socket.socket, int, int] | None:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        sock.connect(sock_path)
        _wl_send(sock, 1, 1, struct.pack("=I", 2))  # wl_display.get_registry(2)
        _wl_send(sock, 1, 0, struct.pack("=I", 3))  # wl_display.sync(3)
        buf = b""
        found: tuple[int, int] | None = None
        deadline = time.monotonic() + timeout
        done = False
        while not done and time.monotonic() < deadline:
            remaining = max(0.0, deadline - time.monotonic())
            readable, _, _ = select.select([sock], [], [], remaining)
            if not readable:
                break
            chunk = sock.recv(65_536)
            if not chunk:
                break
            buf += chunk
            while len(buf) >= 8:
                oid, sz_op = struct.unpack("=II", buf[:8])
                size, opcode = sz_op >> 16, sz_op & 0xFFFF
                if size < 8 or len(buf) < size:
                    break
                body = buf[8:size]
                buf = buf[size:]
                if oid == 2 and opcode == 0 and len(body) >= 8:
                    name, slen = struct.unpack("=II", body[:8])
                    pad = (4 - (slen % 4)) % 4
                    iface = body[8 : 8 + slen - 1].decode("utf-8", "replace")
                    ver_off = 8 + slen + pad
                    if len(body) >= ver_off + 4 and iface == target_iface:
                        (version,) = struct.unpack("=I", body[ver_off : ver_off + 4])
                        found = (name, version)
                elif oid == 3 and opcode == 0:
                    done = True
        if found is None:
            sock.close()
            return None
        return (sock, found[0], found[1])
    except OSError:
        sock.close()
        return None


def _find_wayland_global(sock_path: str, target_iface: str) -> tuple[int, int] | None:
    conn = _wl_connect_and_find_global(sock_path, target_iface)
    if conn is None:
        return None
    sock, name, version = conn
    sock.close()
    return (name, version)


def _kwin_fake_input_key(keycode: int, timeout: float = 2.0) -> None:
    sock_path = _wayland_socket_path()
    if not sock_path:
        raise RuntimeError("WAYLAND_DISPLAY socket not found")
    conn = _wl_connect_and_find_global(sock_path, "org_kde_kwin_fake_input", timeout=timeout)
    if conn is None:
        raise RuntimeError("org_kde_kwin_fake_input global not available")
    sock, name, version = conn
    try:
        bind_payload = (
            struct.pack("=I", name)
            + _wl_str("org_kde_kwin_fake_input")
            + struct.pack("=II", min(version, 4), 4)
        )
        _wl_send(sock, 2, 0, bind_payload)  # wl_registry.bind -> object 4
        _wl_send(sock, 4, 0, _wl_str("koral-bench") + _wl_str("latency benchmark"))  # authenticate
        _wl_send(sock, 4, 10, struct.pack("=II", keycode, 1))  # keyboard_key press
        _wl_send(sock, 4, 10, struct.pack("=II", keycode, 0))  # keyboard_key release
        _wl_send(sock, 1, 0, struct.pack("=I", 5))  # wl_display.sync(5)
        buf = b""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            remaining = max(0.0, deadline - time.monotonic())
            readable, _, _ = select.select([sock], [], [], remaining)
            if not readable:
                raise RuntimeError("timed out waiting for fake-input sync")
            chunk = sock.recv(65_536)
            if not chunk:
                raise RuntimeError("Wayland socket closed during fake-input sync")
            buf += chunk
            while len(buf) >= 8:
                oid, sz_op = struct.unpack("=II", buf[:8])
                size, opcode = sz_op >> 16, sz_op & 0xFFFF
                if size < 8 or len(buf) < size:
                    break
                body = buf[8:size]
                buf = buf[size:]
                if oid == 1 and opcode == 0:  # wl_display.error
                    err_oid, err_code, slen = struct.unpack("=III", body[:12])
                    msg = body[12 : 12 + slen - 1].decode("utf-8", "replace")
                    raise RuntimeError(
                        f"Wayland error on object {err_oid} code {err_code}: {msg}"
                    )
                if oid == 5 and opcode == 0:  # wl_callback.done
                    return
        raise RuntimeError("timed out waiting for fake-input sync")
    finally:
        sock.close()


def _ydotool_socket_exists() -> bool:
    candidates = [
        os.environ.get("YDOTOOL_SOCKET", ""),
        f"/run/user/{os.getuid()}/.ydotool_socket",
        "/tmp/.ydotool_socket",
    ]
    return any(path and os.path.exists(path) for path in candidates)


def detect_capture() -> tuple[str, list[str]] | None:
    wayland = bool(os.environ.get("WAYLAND_DISPLAY"))
    is_kde = "KDE" in os.environ.get("XDG_CURRENT_DESKTOP", "").upper()
    if wayland:
        try:
            if _capture_kwin_screenshot2():
                return "kwin-screenshot2", ["kwin-screenshot2"]
        except Exception:
            pass
    if shutil_which("grim") and not is_kde:
        monitor = os.environ.get("BENCH_OUTPUT", "")
        cmd = ["grim"]
        if monitor:
            cmd += ["-o", monitor]
        cmd += ["-"]
        return "grim-screencopy", cmd
    if wayland and shutil_which("spectacle"):
        return "spectacle", ["spectacle"]
    if shutil_which("grim"):
        monitor = os.environ.get("BENCH_OUTPUT", "")
        cmd = ["grim"]
        if monitor:
            cmd += ["-o", monitor]
        cmd += ["-"]
        return "grim-screencopy", cmd
    if shutil_which("import"):
        return "imagemagick-import", ["import", "-window", "root", "png:-"]
    return None


def detect_injection() -> str | None:
    if os.environ.get("XDG_SESSION_TYPE") == "wayland" and shutil_which("hyprctl"):
        return "hyprland-lua"
    sock_path = _wayland_socket_path()
    if sock_path and _find_wayland_global(sock_path, "org_kde_kwin_fake_input") is not None:
        return "kwin-fake-input"
    if shutil_which("ydotool") and _ydotool_socket_exists():
        return "ydotool"
    if shutil_which("xdotool"):
        return "xdotool"
    return None


def inject_key(method: str, key: str) -> None:
    if method == "hyprland-lua":
        lua = f'hl.dsp.send_key_state({{mods=0,key="{key}",state="down"}}) ; hl.dsp.send_key_state({{mods=0,key="{key}",state="up"}})'
        result = run(["hyprctl", "--batch", f"eval {lua}"], timeout=5)
    elif method == "kwin-fake-input":
        _kwin_fake_input_key(expected_evdev_keycode(key))
        return
    elif method == "ydotool":
        code = expected_evdev_keycode(key)
        result = run(["ydotool", "key", f"{code}:1", f"{code}:0"], timeout=5)
    elif method == "xdotool":
        result = run(["xdotool", "key", key], timeout=5)
    else:
        raise RuntimeError(f"unsupported injection method: {method}")
    if result.returncode != 0:
        raise RuntimeError(f"key injection failed ({result.returncode})")


def capture_hash(cmd: list[str], timeout: float = 3.0) -> str:
    if cmd == ["kwin-screenshot2"]:
        data = _capture_kwin_screenshot2()
        if not data:
            raise RuntimeError("kwin-screenshot2 capture produced no data")
        return hashlib.sha256(data).hexdigest()
    if cmd == ["spectacle"]:
        fd, tmp_path = tempfile.mkstemp(prefix="koral-spectacle-", suffix=".png")
        os.close(fd)
        try:
            result = run(["spectacle", "-b", "-n", "-f", "-o", tmp_path], timeout=timeout)
            if result.returncode != 0:
                raise RuntimeError("capture failed: " + result.stderr.decode("utf-8", "replace")[:200])
            with open(tmp_path, "rb") as handle:
                data = handle.read()
        finally:
            try:
                os.unlink(tmp_path)
            except FileNotFoundError:
                pass
        if not data:
            raise RuntimeError("spectacle capture produced no data")
        return hashlib.sha256(data).hexdigest()
    result = run(cmd, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError("capture failed: " + result.stderr.decode("utf-8", "replace")[:200])
    if not result.stdout:
        raise RuntimeError("capture produced no data")
    return hashlib.sha256(result.stdout).hexdigest()


def wait_for_ready(path: str, timeout: float = 10.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with open(path, "r", encoding="utf-8", errors="replace") as handle:
                if "BENCH_READY" in handle.read():
                    return True
        except OSError:
            pass
        time.sleep(0.02)
    return False


def write_helper(run_dir: str) -> str:
    """Write and return an executable helper generated from this module."""
    os.makedirs(run_dir, exist_ok=True)
    path = os.path.join(run_dir, "pty-key-helper.py")
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(PTY_HELPER_SOURCE)
    os.chmod(path, 0o755)
    return path


def read_ack(path: str) -> dict[str, Any]:
    with open(path, "r", encoding="utf-8") as handle:
        text = handle.read()
    lines = text.splitlines()
    status = lines[0] if lines else ""
    detail: dict[str, Any] = {}
    for line in lines[1:]:
        if line:
            try:
                parsed = json.loads(line)
                if isinstance(parsed, dict):
                    detail = parsed
            except ValueError:
                pass
    return {"status": status, "detail": detail}


def inject_with_latency(
    method: str,
    key: str,
    capture_cmd: list[str],
    ack_path: str,
    expected: bytes,
    timeout: float,
) -> tuple[float, dict[str, Any]]:
    baseline = capture_hash(capture_cmd)
    start_ns = time.perf_counter_ns()
    inject_key(method, key)
    deadline_ns = start_ns + int(timeout * 1_000_000_000)
    ack: dict[str, Any] = {"status": "BENCH_ACK_MISSING", "detail": {}}
    while time.perf_counter_ns() < deadline_ns:
        if ack["status"] == "BENCH_ACK_MISSING":
            try:
                candidate = read_ack(ack_path)
                if candidate["status"] in (KEY_ACK, KEY_TIMEOUT):
                    ack = candidate
            except (OSError, ValueError):
                pass
        if ack["status"] == KEY_ACK:
            if ack["detail"].get("key_hex") != expected.hex():
                raise RuntimeError("PTY acknowledgement names a different key")
            remaining = (deadline_ns - time.perf_counter_ns()) / 1_000_000_000
            if remaining > 0 and capture_hash(capture_cmd, timeout=min(3.0, remaining)) != baseline:
                frame_ns = time.perf_counter_ns()
                if frame_ns < deadline_ns:
                    return ((frame_ns - start_ns) / 1_000_000, ack)
        elif ack["status"] == KEY_TIMEOUT:
            raise RuntimeError("PTY did not receive the injected key before its timeout")
        else:
            time.sleep(0.005)
    if ack["status"] == KEY_TIMEOUT:
        raise RuntimeError("PTY did not receive the injected key before its timeout")
    if ack["status"] != KEY_ACK:
        raise RuntimeError(f"PTY never acknowledged the injected key (ack={ack['status']})")
    raise RuntimeError("PTY acknowledged the key but no frame change was captured")


def measure_once(
    terminal: str,
    binary: str,
    runs: int,
    capture: tuple[str, list[str]],
    injection: str,
    config_args: list[str],
    run_dir: str,
    key: str,
    key_timeout: float,
) -> dict[str, Any]:
    samples: list[float] = []
    key_events: list[dict[str, Any]] = []
    version = terminal_version(binary)
    expected = expected_key_sequence(key)
    capture_name, capture_cmd = capture
    os.makedirs(run_dir, exist_ok=True)
    helper = write_helper(run_dir)
    for index in range(max(1, runs)):
        run_path = os.path.join(run_dir, str(index))
        os.makedirs(run_path, exist_ok=True)
        ready_path = os.path.join(run_path, "ready")
        ack_path = os.path.join(run_path, "ack")
        for path in (ready_path, ack_path):
            try:
                os.unlink(path)
            except FileNotFoundError:
                pass
        env = os.environ.copy()
        env.update(
            BENCH_LAT_READY=ready_path,
            BENCH_LAT_ACK=ack_path,
            BENCH_LAT_KEY_HEX=expected.hex(),
            BENCH_LAT_KEY_TIMEOUT=str(key_timeout),
            BENCH_LAT_HOLD="3",
        )
        with open(os.devnull, "wb") as devnull:
            proc = subprocess.Popen(
                [binary] + config_args + (["-e"] if terminal in {"koral", "alacritty", "ghostty"} else []) + [helper],
                stdout=devnull,
                stderr=devnull,
                env=env,
            )
            try:
                if not wait_for_ready(ready_path, timeout=key_timeout):
                    raise RuntimeError("PTY helper did not write READY marker")
                time.sleep(0.15)
                if injection == "xdotool":
                    windows = run(["xdotool", "search", "--onlyvisible", "--pid", str(proc.pid)])
                    ids = windows.stdout.split()
                    if windows.returncode or len(ids) != 1:
                        raise RuntimeError("could not uniquely identify the X11 benchmark window")
                    focused = run(["xdotool", "windowfocus", "--sync", ids[0].decode()])
                    if focused.returncode:
                        raise RuntimeError("could not focus the X11 benchmark window")
                sample, ack = inject_with_latency(
                    injection, key, capture_cmd, ack_path, expected, key_timeout
                )
                samples.append(round(sample, 3))
                key_events.append({"run": index, **ack})
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait(timeout=3)
    return {
        "terminal": terminal,
        "binary": binary,
        "version": version,
        "methodology": (
            f"capture={capture_name}; injection={injection}; "
            f"pty_key={key} ({expected.hex()}); ack must be {KEY_ACK}; "
            "perf_counter_ns before synthetic key; first changed frame captured after PTY acknowledgement; "
            "linear-interpolated percentiles"
        ),
        "samples_ms": samples,
        "key_events": key_events,
        "p50_ms": percentile(samples, 0.50),
        "p95_ms": percentile(samples, 0.95),
        "p99_ms": percentile(samples, 0.99),
        "status": "ok",
        "reason": "",
    }


def unavailable(
    terminal: str,
    reason: str,
    binary: str = "",
    key_events: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    return {
        "terminal": terminal,
        "binary": binary,
        "version": terminal_version(binary) if binary else "unknown",
        "methodology": "see scripts/bench-latency.py module docstring",
        "samples_ms": [],
        "key_events": key_events or [],
        "p50_ms": None,
        "p95_ms": None,
        "p99_ms": None,
        "status": "unavailable",
        "reason": reason,
    }


def pty_helper_run(
    helper: str,
    ready_path: str,
    ack_path: str,
    key: bytes,
    key_timeout: float,
    feed: bytes | None = None,
    delay: float = 0.05,
) -> tuple[bytes, dict[str, Any], int]:
    """Run the generated helper under a real PTY and return its output/ack."""
    env = os.environ.copy()
    env.update(
        BENCH_LAT_READY=ready_path,
        BENCH_LAT_ACK=ack_path,
        BENCH_LAT_KEY_HEX=key.hex(),
        BENCH_LAT_KEY_TIMEOUT=str(key_timeout),
        BENCH_LAT_HOLD="0.2",
    )
    master, slave = pty.openpty()
    try:
        proc = subprocess.Popen(
            [helper],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            close_fds=True,
        )
        os.close(slave)
        slave = -1
        if not wait_for_ready(ready_path, timeout=key_timeout):
            raise RuntimeError("helper did not write READY under PTY")
        if feed is not None:
            time.sleep(delay)
            os.write(master, feed)
        output = b""
        output_deadline = time.monotonic() + key_timeout + 2

        def drain_output() -> None:
            nonlocal output
            while True:
                readable, _, _ = select.select([master], [], [], 0)
                if not readable:
                    return
                try:
                    chunk = os.read(master, 65_536)
                except OSError:
                    return
                if not chunk:
                    return
                output += chunk

        ack = {"status": "BENCH_ACK_MISSING", "detail": {}}
        deadline = time.monotonic() + key_timeout + 1
        while time.monotonic() < deadline:
            drain_output()
            try:
                ack = read_ack(ack_path)
            except (OSError, ValueError):
                ack = {"status": "BENCH_ACK_MISSING", "detail": {}}
            if ack["status"] in (KEY_ACK, KEY_TIMEOUT):
                break
            time.sleep(0.01)
        while time.monotonic() < output_deadline:
            drain_output()
            if proc.poll() is not None:
                break
            time.sleep(0.01)
        drain_output()
        try:
            status = proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            status = proc.wait()
        return output, ack, status
    finally:
        if slave >= 0:
            os.close(slave)
        os.close(master)


def self_test() -> int:
    assert percentile([], 0.5) is None
    assert percentile([5.0], 0.5) == 5.0
    assert percentile([1.0, 2.0, 3.0, 4.0], 0.5) == 2.5
    assert percentile([1.0, 2.0, 3.0, 4.0], 0.95) == 3.85
    sample = unavailable("test", "self-test")
    assert set(sample) == set(RESULT_KEYS)
    assert expected_key_sequence("F12") == b"\x1b[24~"
    assert expected_evdev_keycode("F12") == 88

    with tempfile.TemporaryDirectory(prefix="koral-pty-selftest-") as directory:
        helper = write_helper(directory)
        assert os.access(helper, os.X_OK)
        key = b"\x1b[24~"

        ready_ok = os.path.join(directory, "ready-ok")
        ack_ok = os.path.join(directory, "ack-ok")
        output_ok, ack, status = pty_helper_run(
            helper, ready_ok, ack_ok, key, 2.0, feed=b"x" + key
        )
        assert ack["status"] == KEY_ACK, ack
        assert ack["detail"]["key_hex"] == key.hex(), ack
        assert status == 0, status
        assert b"48;2;255;0;0m" in output_ok, output_ok[:200]
        assert b"48;2;0;255;0m" in output_ok, output_ok[:200]

        ready_timeout = os.path.join(directory, "ready-timeout")
        ack_timeout = os.path.join(directory, "ack-timeout")
        _, timed_out, timeout_status = pty_helper_run(
            helper, ready_timeout, ack_timeout, key, 0.2, feed=b"x", delay=0
        )
        assert timed_out["status"] == KEY_TIMEOUT, timed_out
        assert timeout_status == 2, timeout_status

        # Harness must refuse a frame change when the PTY reports a timeout.
        timeout_ack = os.path.join(directory, "ack-timeout-harness")
        with open(timeout_ack, "w", encoding="utf-8") as handle:
            handle.write(KEY_TIMEOUT + "\n{}\n")
        original_capture = globals()["capture_hash"]
        original_inject = globals()["inject_key"]
        frames = iter(["baseline", "changed"])
        try:
            globals()["capture_hash"] = lambda cmd, timeout=3.0: next(frames)
            globals()["inject_key"] = lambda method, key: None
            try:
                inject_with_latency("test", "F12", [], timeout_ack, key, 1.0)
                raise AssertionError("timeout sample was accepted")
            except RuntimeError as exc:
                assert "did not receive" in str(exc), exc
        finally:
            globals()["capture_hash"] = original_capture
            globals()["inject_key"] = original_inject

    print(
        json.dumps(
            {
                "status": "self-test-ok",
                "percentile_checks": 4,
                "pty_key_checks": 1,
                "pty_timeout_checks": 1,
                "harness_timeout_checks": 1,
            }
        )
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Measure terminal input-to-photon latency (p50/p95/p99).")
    parser.add_argument("--terminal", default=os.environ.get("BENCH_TERMINAL", "kitty"))
    parser.add_argument("--binary", default="", help="absolute path to the terminal binary used for measurement")
    parser.add_argument("--run-dir", default="", help="directory for the generated PTY helper, ready and ack files")
    parser.add_argument("--runs", type=int, default=1)
    parser.add_argument("--json", default="")
    parser.add_argument("--config", default=os.environ.get("BENCH_CONFIG", ""))
    parser.add_argument("--key", default="F12")
    parser.add_argument("--key-timeout", type=float, default=5.0)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.runs < 1:
        parser.error("--runs must be >= 1")
    if not args.binary or not os.path.isabs(args.binary) or not os.path.isfile(args.binary):
        parser.error("--binary must point to an absolute, existing terminal executable")
    if not os.access(args.binary, os.X_OK):
        parser.error("--binary must be executable")
    if args.key_timeout <= 0:
        parser.error("--key-timeout must be > 0")
    if args.run_dir:
        if not os.path.isabs(args.run_dir):
            parser.error("--run-dir must be an absolute path")
        os.makedirs(args.run_dir, exist_ok=True)
    else:
        args.run_dir = tempfile.mkdtemp(prefix="koral-pty-bench-")
    try:
        expected_key_sequence(args.key)
    except RuntimeError as exc:
        parser.error(str(exc))
    config_args = shlex.split(args.config) if args.config else []
    capture = detect_capture()
    injection = detect_injection()
    if not capture:
        result = unavailable(
            args.terminal,
            "no capture tool (kwin-screenshot2, spectacle, grim, or ImageMagick import) available",
            args.binary,
        )
    elif not injection:
        result = unavailable(
            args.terminal,
            "no input injection tool (hyprctl Lua, kwin-fake-input, ydotool, or xdotool) available",
            args.binary,
        )
    else:
        try:
            result = measure_once(
                args.terminal,
                args.binary,
                args.runs,
                capture,
                injection,
                config_args,
                args.run_dir,
                args.key,
                args.key_timeout,
            )
        except Exception as exc:  # report blockers honestly
            result = unavailable(args.terminal, f"{type(exc).__name__}: {exc}", args.binary)
    payload = json.dumps(result, indent=2, sort_keys=True)
    if args.json:
        with open(args.json + ".tmp", "w", encoding="utf-8") as handle:
            handle.write(payload + "\n")
        os.replace(args.json + ".tmp", args.json)
    else:
        print(payload)
    return 0


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    sys.exit(main())
