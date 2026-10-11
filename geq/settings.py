import json
import os

from gi.repository import GLib

PATH = os.path.join(GLib.get_user_config_dir(), "graphic-eq", "settings.json")

DEFAULTS = {
    "gains": [0.0] * 11,
    "preset": "Flat",
    "eq_on": True,
    "style": "led",        # led | bars | line | area | spectrum
    "theme": "classic",    # classic | amber | ice | neon | rainbow | mono
    "range": 60,           # dB shown below 0 dBFS
    "sensitivity": 0.0,    # dB added to what is drawn
    "fall": "medium",      # slow | medium | fast
    "peak_hold": True,
    "grid": True,
    "db_labels": True,
    "eq_curve": True,
    "labels": "k",         # k | hz | off
    "source": "post",      # post (after EQ) | pre (before EQ)
    "output": None,        # sink name the EQ plays into; None = current default
    "size": [1000, 640],
}


class Settings:
    def __init__(self):
        self.data = dict(DEFAULTS)
        try:
            with open(PATH) as f:
                loaded = json.load(f)
            for k, v in loaded.items():
                if k in DEFAULTS:
                    self.data[k] = v
        except (OSError, ValueError):
            pass
        g = self.data["gains"]
        if not (isinstance(g, list) and len(g) == 11):
            self.data["gains"] = [0.0] * 11
        self._timer = None

    def __getitem__(self, key):
        return self.data[key]

    def __setitem__(self, key, value):
        self.data[key] = value
        self.save_soon()

    def save_soon(self):
        if self._timer is None:
            self._timer = GLib.timeout_add(500, self._flush)

    def _flush(self):
        self._timer = None
        self.save()
        return False

    def save(self):
        try:
            os.makedirs(os.path.dirname(PATH), exist_ok=True)
            tmp = PATH + ".tmp"
            with open(tmp, "w") as f:
                json.dump(self.data, f, indent=2)
            os.replace(tmp, PATH)
        except OSError:
            pass
