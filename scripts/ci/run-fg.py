#!/usr/bin/env python3
"""Run a program in the terminal foreground and record how it and this process
terminated.

tmux reports an empty status for a signal-killed pane and, on this runner, an
empty pane_dead_signal too — so the smoke crash never names its signal. This
wrapper is the reliable alternative: it is the pane process, forks the real
program into its own process group *in the foreground* (so a TUI's tty reads do
not raise SIGTTIN), waits, and writes how the child ended.

It also logs any signal delivered to *itself* (the pane process) and then dies
from it, because the smoke crash turned out to hit the pane process — not the
app, which exits cleanly. That distinction is the whole point of this file.

Usage: run-fg.py <status-file> -- <program> [args...]
"""
import os
import signal
import sys


def main() -> int:
    if len(sys.argv) < 4 or sys.argv[2] != "--":
        sys.stderr.write("usage: run-fg.py <status-file> -- <program> [args...]\n")
        return 2
    status_file = sys.argv[1]
    argv = sys.argv[3:]
    tty = sys.stdin.fileno()

    def note(text: str) -> None:
        with open(status_file, "a", encoding="ascii") as handle:
            handle.write(text + "\n")

    # Record — then die from — any signal delivered to this pane process. The app
    # runs as a child; if the pane dies while the child exits cleanly, this names
    # the signal that is actually killing the smoke test.
    def log_and_die(signum: int, _frame) -> None:
        note(f"pane process got signal {signum} ({signal.Signals(signum).name})")
        signal.signal(signum, signal.SIG_DFL)
        os.kill(os.getpid(), signum)

    for sig in (
        signal.SIGHUP,
        signal.SIGTERM,
        signal.SIGINT,
        signal.SIGQUIT,
        signal.SIGPIPE,
    ):
        signal.signal(sig, log_and_die)
    # Setting the foreground process group writes to the tty; do not get stopped.
    signal.signal(signal.SIGTTOU, signal.SIG_IGN)

    open(status_file, "w", encoding="ascii").close()

    pid = os.fork()
    if pid == 0:
        os.setpgid(0, 0)
        try:
            os.tcsetpgrp(tty, os.getpgid(0))
        except OSError:
            pass
        os.execvp(argv[0], argv)
        os._exit(127)  # unreachable unless execvp fails

    os.setpgid(pid, pid)
    try:
        os.tcsetpgrp(tty, pid)
    except OSError:
        pass

    _, status = os.waitpid(pid, 0)

    try:
        os.tcsetpgrp(tty, os.getpgid(0))
    except OSError:
        pass

    if os.WIFSIGNALED(status):
        sig = os.WTERMSIG(status)
        note(f"child ended: signal {sig} ({signal.Signals(sig).name})")
        return 128 + sig
    code = os.WEXITSTATUS(status)
    note(f"child ended: exit {code}")
    return code


if __name__ == "__main__":
    sys.exit(main())
