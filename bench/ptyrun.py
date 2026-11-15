#!/usr/bin/env python3
"""bench helper: run a command in a 120x40 pseudo-terminal that answers like a real one
(DA1, cursor position, OSC 10/11/4 colour queries). Exit status = the child's."""
import os, pty, sys, select, struct, fcntl, termios, re, time
DEADLINE = time.time() + float(os.environ.get("PTY_TIMEOUT", "15"))
pid, fd = pty.fork()
if pid == 0:
    os.execvp(sys.argv[1], sys.argv[1:])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
buf = b""
while True:
    try:
        r, _, _ = select.select([fd], [], [], 1)
        if time.time() > DEADLINE:
            os.kill(pid, 9); sys.stderr.write("TIMEOUT\n"); break
        if not r:
            continue
            os.kill(pid, 9); break
        data = os.read(fd, 65536)
    except OSError:
        break
    if not data:
        break
    buf = (buf + data)[-256:]
    out = b""
    for m in re.finditer(rb"\x1b\[c|\x1b\[0c|\x1b\[6n|\x1b\]1[01];\?(\x07|\x1b\\)|\x1b\]4;(\d+);\?(\x07|\x1b\\)", data):
        s = m.group(0)
        if s.endswith(b"c"):
            out += b"\x1b[?62;22c"
        elif s.endswith(b"6n"):
            out += b"\x1b[1;1R"
        elif s.startswith(b"\x1b]10"):
            out += b"\x1b]10;rgb:c0c0/c0c0/c0c0\x07"
        elif s.startswith(b"\x1b]11"):
            out += b"\x1b]11;rgb:2020/2020/2020\x07"
        else:
            out += b"\x1b]4;" + m.group(2) + b";rgb:8080/8080/8080\x07"
    if out:
        os.write(fd, out)
_, st = os.waitpid(pid, 0)
sys.exit(os.waitstatus_to_exitcode(st))
