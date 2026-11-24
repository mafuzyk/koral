#!/usr/bin/env python3
"""Measure shell entry and first Wayland buffer submission in an isolated session."""

import argparse
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import signal
import stat
import subprocess
import tempfile
import time


ATTACH = re.compile(rb"-> wl_surface#(\d+)\.attach\(wl_buffer#")
COMMIT = re.compile(rb"-> wl_surface#(\d+)\.commit\(")


def submitted(line, attached):
    if match := ATTACH.search(line):
        attached.add(match[1])
    if match := COMMIT.search(line):
        return match[1] in attached
    return False


def launch_args(terminal, binary, helper):
    if terminal == "kitty":
        return [binary, "--config", "NONE", str(helper)]
    if terminal == "kwi":
        return [binary, str(helper)]
    if terminal == "ghostty":
        return [binary, "--gtk-single-instance=false", "--shell-integration=none", "--linux-cgroup=never",
                f"--command=shell:{shlex.quote(str(helper))}"]
    return [binary, "-e", str(helper)]


def sample(terminal, binary, profile, runtime, display, timeout, prefix):
    marker = profile / "shell-entry"
    clock = "/usr/bin/python3"
    payload = 'import sys,time; print(time.monotonic_ns(), file=open(sys.argv[1], "w"))'
    helper = profile / "launch-shell"
    helper.write_text(
        '#!/bin/sh\nprintf invoked > ' + shlex.quote(str(profile / "probe-invoked"))
        + '\n[ -t 0 ] || exit 1\nexecutable=' + shlex.quote(clock)
        + '\n"$executable" -c ' + shlex.quote(payload) + ' ' + shlex.quote(str(marker))
        + '\nexec /bin/sh -i\n'
    )
    helper.chmod(0o700)
    env = {"PATH": os.defpath, "XDG_RUNTIME_DIR": str(runtime), "WAYLAND_DISPLAY": display,
           "WAYLAND_DEBUG": "client", "HOME": str(profile), "LANG": "C.UTF-8",
           "DBUS_SESSION_BUS_ADDRESS": f"unix:path={runtime / 'startup-bus'}",
           "GTK_A11Y": "none", "NO_AT_BRIDGE": "1"}
    if terminal == "koral" and os.environ.get("KORAL_STARTUP_LOG") == "1":
        env["KORAL_STARTUP_LOG"] = "1"
    if terminal == "ghostty":
        env["SHELL"] = "/bin/sh"
    if terminal == "kwi":
        env["QT_FORCE_STDERR_LOGGING"] = "1"
    if prefix:
        env["LD_LIBRARY_PATH"] = str(prefix / "lib")
        env["XDG_DATA_DIRS"] = f"{prefix / 'share'}:/usr/local/share:/usr/share"
        env["TERMINFO_DIRS"] = f"{prefix / 'share/terminfo'}:/usr/share/terminfo"
        if terminal == "kwi":
            env["QML_IMPORT_PATH"] = str(prefix / "lib/qt6/qml")
        if terminal == "ghostty":
            env["GHOSTTY_RESOURCES_DIR"] = str(prefix / "share/ghostty")
    for key in ("CONFIG", "CACHE", "DATA", "STATE"):
        folder = profile / key.lower()
        folder.mkdir()
        env[f"XDG_{key}_HOME"] = str(folder)
    with (profile / "wayland.log").open("wb") as log, selectors.DefaultSelector() as watcher:
        begin = time.monotonic_ns()
        process = subprocess.Popen(launch_args(terminal, binary, helper), env=env,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                   start_new_session=True)
        first = shell = None
        attached = set()
        pending = b""
        watcher.register(process.stderr, selectors.EVENT_READ)
        try:
            while time.monotonic_ns() - begin < timeout * 1e9:
                for key, _ in watcher.select(0.01):
                    chunk = os.read(key.fd, 32768)
                    now = time.monotonic_ns()
                    if not chunk:
                        watcher.unregister(key.fd)
                        continue
                    log.write(chunk)
                    lines = (pending + chunk).split(b"\n")
                    pending = lines.pop()[-65536:]
                    for line in lines:
                        if first is None and submitted(line, attached):
                            first = (now - begin) / 1e6
                if shell is None:
                    try:
                        value = int(marker.read_text())
                        if value >= begin:
                            shell = (value - begin) / 1e6
                    except (FileNotFoundError, ValueError):
                        pass
                if first is not None and shell is not None or process.poll() is not None:
                    break
        finally:
            exit_before_cleanup = process.poll()
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
    return {"first_buffer_commit_ms": first, "shell_entry_marker_ms": shell,
            "exit_before_cleanup": exit_before_cleanup,
            "status": "ok" if first is not None and shell is not None else "unavailable"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--terminal", choices=("koral", "kitty", "ghostty", "alacritty", "kwi"), default="koral")
    parser.add_argument("--prefix", type=Path, help="absolute relocated package prefix")
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--display")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=5)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        attached = set()
        assert not submitted(b"-> wl_surface#7.commit()", attached)
        assert not submitted(b"-> wl_surface#7.attach(wl_buffer#9, 0, 0)", attached)
        assert not submitted(b"-> wl_surface#8.commit()", attached)
        assert submitted(b"-> wl_surface#7.commit()", attached)
        assert launch_args("kwi", "/kwi", "/probe") == ["/kwi", "/probe"]
        assert launch_args("kitty", "/kitty", "/probe") == ["/kitty", "--config", "NONE", "/probe"]
        return 0
    if args.prefix is not None and (not args.prefix.is_absolute() or not args.prefix.is_dir()):
        parser.error("--prefix must name an absolute directory")
    if not args.binary or not args.binary.is_absolute() or not os.access(args.binary, os.X_OK):
        parser.error("--binary must be an executable absolute path")
    if not args.runtime or not args.runtime.is_absolute() or not args.display or not re.fullmatch(r"wayland-koral-[a-zA-Z0-9-]+", args.display):
        parser.error("provide the explicit runtime and a virtual wayland-koral-* socket")
    runtime = args.runtime.resolve()
    if runtime == Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")).resolve() or runtime == Path(f"/run/user/{os.getuid()}"):
        parser.error("refusing the session runtime directory")
    endpoint = runtime / args.display
    if not endpoint.exists() or not stat.S_ISSOCK(endpoint.stat().st_mode):
        parser.error("the virtual compositor socket does not exist")
    if not args.output or args.runs < 1 or not 0 < args.timeout <= 60:
        parser.error("provide --output, positive --runs and a timeout up to 60 s")
    args.output.mkdir(parents=True, exist_ok=True)
    rows = []
    for _ in range(args.runs):
        profile = Path(tempfile.mkdtemp(dir=args.output.resolve(), prefix="run-"))
        rows.append(sample(args.terminal, str(args.binary), profile, runtime, args.display, args.timeout, args.prefix))
    report = {"binary": str(args.binary), "terminal": args.terminal, "profile": "fresh XDG per run, OS caches not dropped",
              "notes": "First buffer commit is a submission proxy, not scanout. The entry marker is a noninteractive PTY shell probe, not interactive input ACK.",
              "samples": rows}
    (args.output / "startup.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))
    return 0 if all(row["status"] == "ok" for row in rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
