"""Capture an actual X11 window, including host UI, for the palette test."""
import ctypes as c
from pathlib import Path
import struct
import subprocess
import zlib

def capture(window, path):
    geometry = subprocess.check_output(["xdotool", "getwindowgeometry", "--shell", window], text=True)
    values = dict(line.split('=', 1) for line in geometry.splitlines())
    width, height = int(values['WIDTH']), int(values['HEIGHT'])
    x = c.CDLL('libX11.so.6')
    x.XOpenDisplay.argtypes = [c.c_char_p]; x.XOpenDisplay.restype = c.c_void_p
    x.XGetImage.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_int, c.c_uint, c.c_uint, c.c_ulong, c.c_int]
    x.XGetImage.restype = c.c_void_p
    x.XGetPixel.argtypes = [c.c_void_p, c.c_int, c.c_int]; x.XGetPixel.restype = c.c_ulong
    x.XDestroyImage.argtypes = [c.c_void_p]
    x.XCloseDisplay.argtypes = [c.c_void_p]
    x.XDefaultRootWindow.argtypes = [c.c_void_p]; x.XDefaultRootWindow.restype = c.c_ulong
    x.XDefaultScreen.argtypes = [c.c_void_p]; x.XDefaultScreen.restype = c.c_int
    x.XDisplayWidth.argtypes = [c.c_void_p, c.c_int]; x.XDisplayWidth.restype = c.c_int
    x.XDisplayHeight.argtypes = [c.c_void_p, c.c_int]; x.XDisplayHeight.restype = c.c_int
    display = x.XOpenDisplay(None)
    assert display
    image = None
    try:
        screen = x.XDefaultScreen(display)
        width = min(width, x.XDisplayWidth(display, screen) - int(values['X']))
        height = min(height, x.XDisplayHeight(display, screen) - int(values['Y']))
        assert width > 0 and height > 0, 'test window is outside the virtual screen'
        # Read the virtual screen: this also verifies that the host surface was
        # actually presented, rather than only inspecting a window backing store.
        image = x.XGetImage(display, x.XDefaultRootWindow(display), int(values['X']), int(values['Y']), width, height, c.c_ulong(-1), 2)
        assert image
        pixels = [x.XGetPixel(image, col, row) & 0xffffff for row in range(height) for col in range(width)]
        raw = bytes(channel for pixel in pixels for channel in ((pixel >> 16)&255, (pixel >> 8)&255, pixel&255))
        def chunk(kind, data):
            return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind+data))
        scan = b''.join(b'\0' + raw[y*width*3:(y+1)*width*3] for y in range(height))
        Path(path).write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
                              + chunk(b'IDAT', zlib.compress(scan)) + chunk(b'IEND', b''))
        return set(pixels)
    finally:
        if image: x.XDestroyImage(image)
        x.XCloseDisplay(display)
