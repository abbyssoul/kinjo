#!/usr/bin/env python3
"""Run a program in the terminal foreground and record how it terminated.

tmux reports an empty status for a signal-killed pane and, on this runner, an
empty pane_dead_signal too — so the smoke crash never names its signal. This
wrapper is the reliable alternative: it is the pane process, forks the real
program into its own process group *in the foreground* (so a TUI's tty reads do
not raise SIGTTIN), waits, and writes "signal N" or "exit N" to a file. It then
exits non-zero on abnormal termination so the smoke test still fails.

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

    # Setting the foreground process group writes to the tty; the parent must not
    # be stopped by SIGTTOU while doing so.
    signal.signal(signal.SIGTTOU, signal.SIG_IGN)

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

    # Take the controlling terminal back so tmux/the driver behave normally.
    try:
        os.tcsetpgrp(tty, os.getpgid(0))
    except OSError:
        pass

    with open(status_file, "w", encoding="ascii") as handle:
        if os.WIFSIGNALED(status):
            sig = os.WTERMSIG(status)
            handle.write(f"signal {sig} ({signal.Signals(sig).name})\n")
            return 128 + sig
        code = os.WEXITSTATUS(status)
        handle.write(f"exit {code}\n")
        return code


if __name__ == "__main__":
    sys.exit(main())
