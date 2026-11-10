"""libfiles — talk to file FORCE from a Quick Action script.

    import libfiles
    for path in libfiles.selection():
        ...
    answers = libfiles.ask("Resize", [libfiles.choice("size", "Width", ["1080", "720"])])
    libfiles.made(new_path)
    libfiles.notify("Done", "3 pictures resized")

Messages go to Files as JSON lines on stdout; answers come back on stdin.
"""
import json
import os
import sys

__all__ = ["selection", "answer", "ask", "choice", "text", "number", "notify", "made", "reveal", "progress", "Cancelled"]


class Cancelled(Exception):
    """The user closed the dialog without running."""


def _send(message):
    sys.stdout.write("@files " + json.dumps(message) + "\n")
    sys.stdout.flush()


def selection():
    """The selected files and folders, as paths."""
    return list(sys.argv[1:])


def answer(key, default=None):
    """An answer from the action's own [[ask]] dialog (asked before it ran)."""
    return os.environ.get("FILES_ANSWER_" + key.upper(), default)


def choice(key, label, options):
    return {"kind": "choice", "key": key, "label": label, "options": list(options)}


def text(key, label, default=""):
    return {"kind": "text", "key": key, "label": label, "default": default}


def number(key, label, default=0):
    return {"kind": "number", "key": key, "label": label, "default": str(default)}


def ask(title, fields):
    """Shows a dialog in Files and waits; returns {key: value}. Raises Cancelled."""
    _send({"op": "ask", "title": title, "fields": fields})
    reply = json.loads(sys.stdin.readline() or '{"cancelled": true}')
    if reply.get("cancelled"):
        raise Cancelled()
    return reply.get("answers", {})


def notify(title, body=None, level="success"):
    """A notification in Files: info, success, warning or error."""
    _send({"op": "notify", "title": title, "body": body, "level": level})


def made(path):
    """A file this script made or changed (offered to Reveal / Open)."""
    _send({"op": "made", "path": os.path.abspath(path)})


def reveal(path):
    """Opens the file's folder in Files, the file selected."""
    _send({"op": "reveal", "path": os.path.abspath(path)})


def progress(text):
    """What it is doing now (DevTools → Tasks, the Console)."""
    _send({"op": "progress", "text": text})
