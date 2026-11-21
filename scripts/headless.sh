#!/usr/bin/env bash
# Runs a command on an invisible Wayland display, so test windows never appear on the desktop
# or take its focus. Usage: headless.sh timeout 240 path/to/wubble --smoke page
#
# Each call gets its own headless mutter (one virtual monitor, 1360x880 by default) on its own
# private D-Bus session, torn down when the command exits: two callers' windows never take
# keyboard focus or the clipboard from each other, and nothing reaches the desktop's display,
# D-Bus session, clipboard or settings. In that compositor:
# - a seat with a keyboard: a RemoteDesktop session (org.gnome.Mutter.RemoteDesktop, on the
#   private bus) adds mutter's virtual input devices and presses Shift once, so a mapped window
#   gets keyboard focus and an input serial, which the clipboard (wl_data_device.set_selection)
#   needs. The pointer is never moved (no hover in screenshots).
# - a ticker: a small GTK window (software-drawn, plain wl_shm buffers) that draws every frame,
#   under the command's windows. Without it mutter 50's headless frame clock can park
#   ("scheduled-later") on a commit-timing commit (NVIDIA's Vulkan and EGL presents use
#   wp_commit_timer_v1 + wp_fifo_v1) after an empty frame and never wake: the window gets no
#   frame callback again and stops drawing. A desktop always has something else damaging the
#   stage. The ticker maps before the command starts, so the command's first window takes focus.
# - Xwayland: mutter's own, as on the desktop (arboard's image clipboard falls back to X11 on
#   GNOME, which offers no data-control protocol); the command gets its DISPLAY and XAUTHORITY.
# The command keeps the caller's environment except WAYLAND_DISPLAY, DISPLAY, XAUTHORITY,
# XDG_SESSION_TYPE and its D-Bus session: it gets a session bus address nothing listens on, so
# what it would ask the desktop for (the keyring, the file manager, notifications, portals)
# fails as on a machine without them instead of reaching the owner's desktop.
# WUBBLE_HEADLESS_BUS=ADDRESS gives it that bus instead (a private one the caller started).
# Env: WUBBLE_HEADLESS_SIZE (default 1360x880); WUBBLE_HEADLESS_LOG=FILE keeps the compositor's
# output there (default: a file in $XDG_RUNTIME_DIR, removed afterwards).
set -u

# The whole script is read before anything runs (functions, then one line), so replacing or
# editing this file never disturbs a call that is already running.

# Inside dbus-run-session: mutter and the helper (seat, X11, ticker); either ending ends both.
compositor() {
  local name=$1 ready=$2 owner=$3 size=$4 log=$5
  echo $$ >"$ready.pid"
  mutter --headless --wayland --virtual-monitor "$size" --wayland-display "$name" &
  local mutter=$!
  # Mutter first, alone: it then removes its X11 sockets and locks (a TERM to the whole group
  # leaves them in /tmp).
  local base=${ready%.ready}
  trap 'kill -TERM "$mutter" 2>/dev/null; wait "$mutter"; kill $(jobs -p) 2>/dev/null; wait
    rm -f "$ready" "$ready.pid" "$base.bus.conf"; [ "$log" != "$base.log" ] || rm -f "$log"
    exit 0' TERM INT HUP
  WAYLAND_DISPLAY=$name GDK_BACKEND=wayland GSK_RENDERER=cairo GTK_A11Y=none \
    python3 -I - "$ready" "$owner" "$log" "$mutter" <<'PY' &
import os, re, socket, sys, time
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gio, GLib

ready, owner, log, mutter = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
RD = "org.gnome.Mutter.RemoteDesktop"


def call(dest, path, iface, method, args=None, sig=None):
    reply = GLib.VariantType(sig) if sig else None
    return bus.call_sync(dest, path, iface, method, args, reply, 0, 5000, None).unpack()


def has_rd():
    return call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                "NameHasOwner", GLib.Variant("(s)", (RD,)), "(b)")[0]


sock = os.path.join(os.environ["XDG_RUNTIME_DIR"], os.environ["WAYLAND_DISPLAY"])
for _ in range(400):
    if os.path.exists(sock) and has_rd():
        break
    time.sleep(0.05)
else:
    sys.exit("headless.sh: mutter never came up")

# The seat: its keyboard comes with the first virtual key event; Shift alone types nothing.
session = call(RD, "/org/gnome/Mutter/RemoteDesktop", RD, "CreateSession", None, "(o)")[0]
call(RD, session, RD + ".Session", "Start")
for down in (True, False):
    call(RD, session, RD + ".Session", "NotifyKeyboardKeysym",
         GLib.Variant("(ub)", (0xFFE1, down)))



def x11():
    """Mutter's own Xwayland (the desktop has one: arboard's image clipboard falls back to X11
    on GNOME, which has no data-control protocol): its display from mutter's log, started by a
    first connection, its cookie file from its command line."""
    for _ in range(400):
        with open(log, errors="replace") as f:
            found = re.search(r"Using public X11 display (:\d+)", f.read())
        if found:
            break
        time.sleep(0.05)
    else:
        sys.exit("headless.sh: mutter never named its X11 display")
    display = found.group(1)
    wake = socket.socket(socket.AF_UNIX)
    wake.connect("/tmp/.X11-unix/X" + display[1:])
    for _ in range(400):
        for pid in filter(str.isdigit, os.listdir("/proc")):
            try:
                with open(f"/proc/{pid}/stat") as f:
                    stat = f.read()
                with open(f"/proc/{pid}/cmdline", "rb") as f:
                    args = f.read().decode(errors="replace").split("\0")
            except OSError:
                continue
            if stat.rsplit(")", 1)[1].split()[1] == mutter and "-auth" in args:
                wake.close()
                return display, args[args.index("-auth") + 1]
        time.sleep(0.05)
    sys.exit("headless.sh: Xwayland never started")


display, cookie = x11()

# The ticker; the display is ready once it has drawn twice (mapped, its frames running).
from gi.repository import Gtk  # only now: importing it opens the display

Gtk.init()
frames = [0]


def draw(_area, cr, _w, _h):
    frames[0] += 1
    cr.set_source_rgb(frames[0] % 2 * 0.1, 0, 0)
    cr.paint()
    if frames[0] == 2:
        with open(ready + ".tmp", "w") as f:
            f.write(f"{display}\n{cookie}\n")
        os.rename(ready + ".tmp", ready)


area = Gtk.DrawingArea()
area.set_draw_func(draw)
area.add_tick_callback(lambda w, _clock: w.queue_draw() or True)
ticker = Gtk.Window(title="headless ticker", default_width=16, default_height=16)
ticker.set_child(area)
ticker.present()

loop = GLib.MainLoop()
quit = lambda *_: loop.quit()
bus.signal_subscribe(RD, RD + ".Session", "Closed", session, None, 0, quit)
Gio.bus_watch_name_on_connection(bus, RD, 0, None, quit)


def owner_alive():
    try:
        os.kill(owner, 0)
        return True
    except ProcessLookupError:
        loop.quit()
        return False


GLib.timeout_add_seconds(1, owner_alive)
loop.run()
PY
  wait -n
  kill -TERM $$
}

main() {
  [ $# -gt 0 ] || { echo "usage: headless.sh <command...>" >&2; exit 2; }
  : "${XDG_RUNTIME_DIR:?}"
  name=wubble-headless-$$
  ready=$XDG_RUNTIME_DIR/$name.ready
  bus=$XDG_RUNTIME_DIR/$name.bus.conf
  log=${WUBBLE_HEADLESS_LOG:-$XDG_RUNTIME_DIR/$name.log}
  xdisplay='' xauth=''
  rm -f "$ready"
  # The private bus starts no services (no portals, keyring, GVfs: they would reach the desktop's).
  cat >"$bus" <<'XML'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
XML
  # The compositor's world: no desktop display or D-Bus session, no GVfs, settings in memory only.
  env -u DISPLAY -u WAYLAND_DISPLAY -u DBUS_SESSION_BUS_ADDRESS \
    GIO_USE_VFS=local GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 XDG_SESSION_TYPE=wayland \
    GDK_DEBUG=no-portals GTK_USE_PORTAL=0 \
    setsid dbus-run-session --config-file="$bus" -- "$BASH" "$0" --compositor "$name" "$ready" $$ \
    "${WUBBLE_HEADLESS_SIZE:-1360x880}" "$log" >"$log" 2>&1 </dev/null &
  comp=$!
  # Ends the compositor's whole session (bus, mutter, helper) and removes what it leaves behind.
  stop() {
    [ -s "$ready.pid" ] && kill -TERM "$(cat "$ready.pid")" 2>/dev/null
    for _ in $(seq 100); do kill -0 -- "-$comp" 2>/dev/null || break; sleep 0.05; done
    kill -TERM -- "-$comp" 2>/dev/null
    for _ in $(seq 20); do kill -0 -- "-$comp" 2>/dev/null || break; sleep 0.05; done
    kill -KILL -- "-$comp" 2>/dev/null
    wait "$comp" 2>/dev/null
    rm -f "$ready" "$ready.pid" "$bus" "$XDG_RUNTIME_DIR/$name" "$XDG_RUNTIME_DIR/$name.lock" ${xauth:+"$xauth"}
    [ -n "${WUBBLE_HEADLESS_LOG-}" ] || rm -f "$log"
  }
  trap 'stop; exit 143' TERM INT HUP
  for _ in $(seq 400); do
    [ -e "$ready" ] && break
    kill -0 "$comp" 2>/dev/null || break
    sleep 0.05
  done
  if ! [ -e "$ready" ]; then
    echo "headless.sh: no headless display (WUBBLE_HEADLESS_LOG=FILE keeps the reason)" >&2
    stop
    exit 125
  fi
  { read -r xdisplay; read -r xauth; } <"$ready"
  # In the background, so a signal to this script is handled at once and passed on.
  env WAYLAND_DISPLAY="$name" DISPLAY="$xdisplay" XAUTHORITY="$xauth" XDG_SESSION_TYPE=wayland \
    DBUS_SESSION_BUS_ADDRESS="${WUBBLE_HEADLESS_BUS:-unix:path=/nonexistent/wubble-headless-bus}" \
    GIO_USE_VFS=local GTK_USE_PORTAL=0 \
    "$@" <&0 &
  cmd=$!
  trap 'kill -TERM "$cmd" 2>/dev/null; wait "$cmd"; stop; exit 143' TERM INT HUP
  wait "$cmd"
  rc=$?
  stop
  exit "$rc"
}

if [ "${1-}" = --compositor ]; then shift; compositor "$@"; else main "$@"; fi; exit
