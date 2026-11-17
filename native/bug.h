// Generated from native/abi.toml by scripts/generate-abi. Edit the definition instead.
#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BUG_ABI_VERSION 3

// All calls belong to the initializing thread. Copy borrowed text before returning from the
// callback. Reentry into Bug from the callback is unsupported.
typedef void (*BugEvent)(void* user, uint32_t kind, uint64_t view, const char* text, size_t length);

enum {
    BUG_OPENED = 1,
    BUG_CLOSED = 2,
    BUG_TITLE = 3,
    BUG_URL = 4,
    BUG_LOADED = 5,
    BUG_FRAME = 6,
    BUG_HOOK = 7,
    BUG_ERROR = 8,
    BUG_EVALUATED = 9,
    BUG_CLIPBOARD = 10,
    BUG_FILE_PICKER = 11,
};

enum {
    BUG_BACK = 1,
    BUG_FORWARD = 2,
    BUG_RELOAD = 3,
    BUG_STOP = 4,
    BUG_CLOSE = 5,
    BUG_EVAL = 6,
    BUG_ZOOM = 7,
    BUG_FOCUS = 8,
    BUG_BLUR = 9,
    BUG_COPY = 10,
    BUG_CUT = 11,
    BUG_PASTE = 12,
};

uint32_t bug_abi_version(void);
int bug_init(const char* executable_path, const char* profile_path, BugEvent callback, void* user);
void bug_shutdown(void);
void bug_pump(void);
uint64_t bug_open(const char* url);
int bug_navigate(uint64_t view, const char* url);
int bug_evaluate(uint64_t view, uint64_t request, const char* function_body);
// Returns 1 on success, 0 for a stale request, -1 for too many files, or -2 for an unreadable
// file.
int bug_reply_file_picker(uint64_t view, uint64_t request, const char* const* paths, size_t count);
int bug_action(uint64_t view, uint32_t action, const char* text, double number);
int bug_resize(uint64_t view, uint32_t width, uint32_t height, double scale);
// Returns the pixel count, or zero without a CPU frame. With insufficient capacity, writes only
// the dimensions. Pixels are packed as 0x00RRGGBB.
size_t bug_frame(uint64_t view, uint32_t* pixels, size_t capacity, uint32_t* width, uint32_t* height);
// Mouse kinds are move=0, down=1, up=2, wheel=3, leave=4. DOM button masks are left=1, right=2,
// middle=4, back=8, forward=16.
int bug_mouse(uint64_t view, uint32_t kind, int32_t x, int32_t y, uint32_t button, uint32_t buttons, uint32_t modifiers, double dx, double dy, uint32_t clicks);
// Modifier bits are alt=1, ctrl=2, shift=4, super=8. Key names use Ladybird symbolic names.
// Unknown names map to Invalid.
int bug_key(uint64_t view, int pressed, const char* key, uint32_t codepoint, uint32_t modifiers, int repeat);

#ifdef __cplusplus
}
#endif
