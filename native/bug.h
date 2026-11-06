#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// All calls, including destruction, belong to the initializing thread. Callbacks
// must copy their borrowed text before returning and must not call back into Bug.
// The ABI is deliberately small; no AK, C++, or Rust-owned types cross it.
typedef void (*BugEvent)(void*, uint32_t kind, uint64_t view, const char* text, size_t length);
enum { BUG_OPENED = 1, BUG_CLOSED, BUG_TITLE, BUG_URL, BUG_LOADED, BUG_FRAME, BUG_HOOK, BUG_ERROR, BUG_EVALUATED, BUG_CLIPBOARD };
enum { BUG_BACK = 1, BUG_FORWARD, BUG_RELOAD, BUG_STOP, BUG_CLOSE, BUG_EVAL, BUG_ZOOM, BUG_FOCUS, BUG_BLUR, BUG_COPY, BUG_CUT, BUG_PASTE };

uint32_t bug_abi_version(void);
int bug_init(const char* executable_path, const char* profile_path, BugEvent, void* user);
void bug_shutdown(void);
void bug_pump(void);
uint64_t bug_open(const char* url);
int bug_navigate(uint64_t view, const char* url);
int bug_evaluate(uint64_t view, uint64_t request, const char* function_body);
int bug_action(uint64_t view, uint32_t action, const char* text, double number);
int bug_resize(uint64_t view, uint32_t width, uint32_t height, double scale);
// Returns the number of pixels, or zero if there is no CPU frame. When capacity
// is too small, only the dimensions are written. Pixels are packed 0x00RRGGBB.
size_t bug_frame(uint64_t view, uint32_t* pixels, size_t capacity, uint32_t* width, uint32_t* height);
// Mouse kinds: move=0, down=1, up=2, wheel=3, leave=4. Button masks follow DOM
// buttons: left=1, right=2, middle=4, back=8, forward=16.
int bug_mouse(uint64_t view, uint32_t kind, int32_t x, int32_t y, uint32_t button,
    uint32_t buttons, uint32_t modifiers, double dx, double dy, uint32_t clicks);
// Modifier bits: alt=1, ctrl=2, shift=4, super=8. Key names use Ladybird's
// symbolic names; unknown names safely map to Invalid.
int bug_key(uint64_t view, int pressed, const char* key, uint32_t codepoint,
    uint32_t modifiers, int repeat);

#ifdef __cplusplus
}
#endif
