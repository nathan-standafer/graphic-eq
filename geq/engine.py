"""The equalizer itself: a PipeWire filter-chain exposed as a virtual sink.

We run it as a small `pipewire -c` client process. It creates the sink
"Graphic EQ" (geq_sink); audio sent there goes through 11 peaking biquads per
channel and out to the chosen real output. Gains are changed live with pw-cli.
"""

import json
import os
import subprocess
import tempfile
import threading
import time

from . import bands
from .routing import VIRTUAL_SINK


def _config(target):
    nodes, links = [], []
    for ch in "LR":
        for i, f in enumerate(bands.FREQS):
            nodes.append(
                f'{{ type=builtin name=eq{ch}{i} label=bq_peaking '
                f'control={{ "Freq"={f} "Q"={bands.Q} "Gain"=0.0 }} }}'
            )
            if i:
                links.append(f'{{ output="eq{ch}{i-1}:Out" input="eq{ch}{i}:In" }}')
    last = bands.COUNT - 1
    return f"""context.properties = {{ log.level = 0 }}
context.spa-libs = {{ audio.convert.* = audioconvert/libspa-audioconvert  support.* = support/libspa-support }}
context.modules = [
  {{ name = libpipewire-module-rt args = {{ }} flags = [ ifexists nofail ] }}
  {{ name = libpipewire-module-protocol-native }}
  {{ name = libpipewire-module-client-node }}
  {{ name = libpipewire-module-adapter }}
  {{ name = libpipewire-module-filter-chain args = {{
      node.description = "Graphic EQ"
      media.name = "Graphic EQ"
      filter.graph = {{
        nodes = [ {' '.join(nodes)} ]
        links = [ {' '.join(links)} ]
        inputs = [ "eqL0:In" "eqR0:In" ]
        outputs = [ "eqL{last}:Out" "eqR{last}:Out" ]
      }}
      audio.channels = 2
      audio.position = [ FL FR ]
      capture.props = {{ node.name = "{VIRTUAL_SINK}" media.class = Audio/Sink node.always-process = true node.pause-on-idle = false }}
      playback.props = {{ node.name = "geq_out" node.passive = true node.always-process = true node.pause-on-idle = false target.object = {json.dumps(target)} }}
  }} }}
]
"""


def _find_node_id(name):
    try:
        out = subprocess.run(["pw-dump"], capture_output=True, text=True, timeout=3).stdout
        for o in json.loads(out):
            if o.get("type") == "PipeWire:Interface:Node" and \
               o.get("info", {}).get("props", {}).get("node.name") == name:
                return o["id"]
    except (OSError, subprocess.SubprocessError, ValueError):
        pass
    return None


class EqEngine:
    def __init__(self):
        self.proc = None
        self.node_id = None
        self._conf = None
        self._gains = None
        self._cv = threading.Condition()
        self._dirty = False
        self._alive = False

    @property
    def running(self):
        return self.proc is not None and self.proc.poll() is None and self.node_id is not None

    def start(self, target, timeout=6.0):
        """Blocking: launch the filter-chain and wait for its sink. Run off the UI thread."""
        self.stop()
        rundir = os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir()
        self._conf = os.path.join(rundir, "graphic-eq-engine.conf")
        with open(self._conf, "w") as f:
            f.write(_config(target))
        try:
            self.proc = subprocess.Popen(
                ["pipewire", "-c", self._conf],
                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
            )
        except OSError as e:
            self.proc = None
            return f"could not launch pipewire: {e}"
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                err = (self.proc.stderr.read() or "").strip().splitlines()
                self.proc = None
                return err[-1] if err else "engine exited"
            nid = _find_node_id(VIRTUAL_SINK)
            if nid is not None:
                self.node_id = nid
                self._alive = True
                threading.Thread(target=self._sender, daemon=True).start()
                if self._gains:
                    self.set_gains(self._gains)
                return None
            time.sleep(0.15)
        self.stop()
        return "timed out waiting for the EQ sink"

    def stop(self):
        with self._cv:
            self._alive = False
            self._cv.notify_all()
        if self.proc is not None:
            self.proc.terminate()
            try:
                self.proc.wait(2)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self.proc = None
        self.node_id = None

    def set_gains(self, gains):
        """Latest-wins: slider drags coalesce into few pw-cli calls."""
        with self._cv:
            self._gains = list(gains)
            self._dirty = True
            self._cv.notify()

    def _sender(self):
        while True:
            with self._cv:
                while self._alive and not self._dirty:
                    self._cv.wait()
                if not self._alive:
                    return
                gains, nid = self._gains, self.node_id
                self._dirty = False
            params = " ".join(
                f'"eq{ch}{i}:Gain" {g:.2f}' for i, g in enumerate(gains) for ch in "LR"
            )
            try:
                subprocess.run(
                    ["pw-cli", "s", str(nid), "Props", f"{{ params = [ {params} ] }}"],
                    capture_output=True, timeout=3,
                )
            except (OSError, subprocess.SubprocessError):
                pass
            time.sleep(0.02)
