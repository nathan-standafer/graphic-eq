"""Thin wrappers over pactl (works on PipeWire-pulse and PulseAudio)."""

import json
import subprocess

VIRTUAL_SINK = "geq_sink"


def _run(*args, timeout=3):
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
        return r.stdout if r.returncode == 0 else None
    except (OSError, subprocess.SubprocessError):
        return None


def default_sink():
    out = _run("pactl", "get-default-sink")
    return out.strip() if out else None


def set_default_sink(name):
    return _run("pactl", "set-default-sink", name) is not None


def list_sinks():
    """[(name, description)] for real output devices (our own sink is hidden)."""
    out = _run("pactl", "--format=json", "list", "sinks")
    if not out:
        return []
    try:
        sinks = json.loads(out)
    except ValueError:
        return []
    return [
        (s["name"], s.get("description") or s["name"])
        for s in sinks
        if not s["name"].startswith("geq_")
    ]


def _sink_index(name):
    out = _run("pactl", "--format=json", "list", "sinks")
    try:
        for s in json.loads(out or "[]"):
            if s["name"] == name:
                return s["index"]
    except ValueError:
        pass
    return None


def move_streams(src, dst):
    """Move playing streams from sink src to sink dst (either may be None = all -> dst)."""
    out = _run("pactl", "--format=json", "list", "sink-inputs")
    if not out:
        return
    src_idx = _sink_index(src) if src else None
    try:
        inputs = json.loads(out)
    except ValueError:
        return
    for i in inputs:
        if src_idx is not None and i.get("sink") != src_idx:
            continue
        if i.get("properties", {}).get("node.name", "").startswith("geq_"):
            continue  # never move the EQ's own output stream (it would loop into itself)
        _run("pactl", "move-sink-input", str(i["index"]), dst)
