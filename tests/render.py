#!/usr/bin/env python3
"""Exercise the real native ABI and assert that Ladybird produces nonblank pixels."""
import ctypes as c
from pathlib import Path
import tempfile
import time

root = Path(__file__).resolve().parents[1]
lib = c.CDLL(str(root / "build/engine/lib/libbug_engine.so"))
events = []
callback_type = c.CFUNCTYPE(None, c.c_void_p, c.c_uint32, c.c_uint64, c.c_void_p, c.c_size_t)

@callback_type
def callback(user, kind, view, text, size):
    events.append((kind, view, c.string_at(text, size).decode('utf-8', errors='replace') if size else ''))

lib.bug_abi_version.restype = c.c_uint32
lib.bug_init.argtypes = [c.c_char_p, c.c_char_p, callback_type, c.c_void_p]
lib.bug_init.restype = c.c_int
lib.bug_open.argtypes = [c.c_char_p]
lib.bug_open.restype = c.c_uint64
lib.bug_frame.argtypes = [c.c_uint64, c.POINTER(c.c_uint32), c.c_size_t, c.POINTER(c.c_uint32), c.POINTER(c.c_uint32)]
lib.bug_frame.restype = c.c_size_t
lib.bug_pump.restype = None
lib.bug_shutdown.restype = None

with tempfile.TemporaryDirectory(prefix='bug-render-') as profile:
    assert lib.bug_abi_version() == 2
    assert lib.bug_init(str(root / 'build/engine/bin/bug').encode(), profile.encode(), callback, None), events
    try:
        view = lib.bug_open((root / 'examples/smoke.html').as_uri().encode())
        assert view
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            lib.bug_pump()
            assert not any(e[0] == 8 for e in events), events[-10:]
            if any(e[0] == 5 and e[1] == view for e in events):
                width, height = c.c_uint32(), c.c_uint32()
                count = lib.bug_frame(view, None, 0, c.byref(width), c.byref(height))
                if count:
                    assert count == width.value * height.value and count <= 64 * 1024 * 1024
                    pixels = (c.c_uint32 * count)()
                    assert lib.bug_frame(view, pixels, count, c.byref(width), c.byref(height)) == count
                    # Fixture background and animated rectangle: rendering, not
                    # merely successful navigation or an empty native window.
                    colors = set(pixels)
                    if 0xeef3eb in colors and 0x577c50 in colors:
                        with open(root / 'build/smoke.ppm', 'wb') as out:
                            out.write(f'P6\n{width.value} {height.value}\n255\n'.encode())
                            out.write(bytes(channel for p in pixels for channel in ((p >> 16)&255, (p >> 8)&255, p&255)))
                        print(f'PASS: Ladybird produced {width.value}x{height.value} pixels with expected page colors')
                        break
            time.sleep(0.008)
        else:
            raise AssertionError(f'no rendered fixture frame; last events: {events[-15:]}')
    finally:
        lib.bug_shutdown()
