"""Host-side, opt-in transport probe; all fixture code executes in the VM."""
import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import time

backend, guest = sys.argv[1:3]
expected = sys.argv[3].encode() if len(sys.argv) > 3 else b"bench offline fixture: OK"

def session(command, interrupt=False):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    original = termios.tcgetattr(slave)
    process = subprocess.Popen([backend, "exec", "-it", "--workdir", "/workspace", guest,
                                *command], stdin=slave, stdout=slave, stderr=slave,
                               start_new_session=True)
    output = bytearray()
    sent = False
    try:
        deadline = time.monotonic() + 12  # Diagnostic fixture only; never an agent budget.
        while process.poll() is None and time.monotonic() < deadline:
            ready, _, _ = select.select([master], [], [], 0.05)
            if ready:
                output.extend(os.read(master, 8192))
                assert len(output) < 65536, "unexpected diagnostic output volume"
            if interrupt and not sent and b"BENCH_READY" in output:
                os.write(master, b"\x03")
                sent = True
        assert process.poll() is not None, "terminal fixture transport did not finish"
        # Drain bytes already delivered, without waiting on a still-open slave.
        while select.select([master], [], [], 0)[0]:
            output.extend(os.read(master, 8192))
            assert len(output) < 65536
        assert termios.tcgetattr(slave) == original, "sbx failed to restore terminal state"
        if interrupt:
            assert sent and b"BENCH_INTERRUPTED" in output
            assert process.returncode != 0
        else:
            assert process.returncode == 0 and b"40 120" in output
            assert expected in output
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        termios.tcsetattr(slave, termios.TCSANOW, original)
        os.close(master)
        os.close(slave)

session(["sh", "-c", "stty rows 40 cols 120; test -t 0 && test -t 1 && stty size; "
         "cargo --config /replay/config.toml run --release --frozen"])
session(["python3", "-u", "-c", "import time\nprint('BENCH_READY')\ntry: time.sleep(60)\n"
         "except KeyboardInterrupt:\n print('BENCH_INTERRUPTED')\n raise SystemExit(130)"], True)
print("terminal transport verified")
